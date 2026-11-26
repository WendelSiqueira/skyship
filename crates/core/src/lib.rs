//! skyship-core: toda a lógica de deploy, sem interface.
//!
//! O CLI e a GUI chamam as mesmas funções e recebem o andamento como
//! [`progress::Event`]; nada aqui escreve no terminal.

pub mod archive;
pub mod config;
pub mod docker;
pub mod ec2;
pub mod ecr;
pub mod lightsail;
pub mod local;
pub mod login;
pub mod progress;
pub mod project;
pub mod regions;
pub mod scan;

use anyhow::Result;
use config::{Config, Target};
use progress::{Event, Progress};
use serde::Serialize;

/// Conexão com a AWS (região + perfil opcional).
pub struct Aws {
    pub sdk: aws_config::SdkConfig,
}

impl Aws {
    /// `profile`: nome em `~/.aws/credentials`; `None` usa a cadeia padrão do SDK.
    pub async fn connect(region: &str, profile: Option<&str>) -> Self {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region.to_string()));
        if let Some(p) = profile {
            loader = loader.profile_name(p);
        }
        Self {
            sdk: loader.load().await,
        }
    }

    /// Número da conta AWS; falha se as credenciais forem inválidas ou estiverem ausentes.
    pub async fn account(&self) -> Result<String> {
        let id = aws_sdk_sts::Client::new(&self.sdk)
            .get_caller_identity()
            .send()
            .await
            .map_err(|e| {
                anyhow::anyhow!("credenciais AWS inválidas ou ausentes (AWS_PROFILE / AWS_ACCESS_KEY_ID...): {e}")
            })?;
        Ok(id.account().unwrap_or_default().to_string())
    }
}

/// Resultado de um deploy bem-sucedido.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub url: Option<String>,
}

/// Uma instância (EC2) ou serviço (Lightsail) do app.
#[derive(Debug, Clone, Serialize)]
pub struct StatusEntry {
    pub id: String,
    pub state: String,
    pub url: Option<String>,
}

/// Um app (container service do Lightsail ou instância EC2 do skyship) encontrado na conta.
#[derive(Debug, Clone, Serialize)]
pub struct AppSummary {
    pub name: String,
    pub target: Target,
    pub state: String,
    pub url: Option<String>,
    pub region: String,
    /// Tamanho em uso: `power` do Lightsail (ex.: `nano`) ou tipo da instância EC2.
    pub size: Option<String>,
    /// Criado ou marcado pelo skyship (tag `skyship:name`). Os outros foram criados por fora.
    pub managed: bool,
    /// Só na EC2: resumo dos status checks da instância (`ok`, `initializing`, `impaired`...).
    pub checks: Option<String>,
}

/// Resultado de [`list_apps`]: o que foi achado e o que não foi possível consultar.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ListResult {
    pub apps: Vec<AppSummary>,
    /// Um aviso por destino que falhou (ex.: falta de permissão), sem derrubar a listagem.
    pub warnings: Vec<String>,
}

/// Lista os apps da região de `aws`. Não falha por inteiro: se um destino der erro (por exemplo,
/// o usuário só tem permissão de Lightsail), o erro vira um aviso e o outro destino continua.
pub async fn list_apps(aws: &Aws) -> ListResult {
    let region = aws.sdk.region().map(|r| r.to_string()).unwrap_or_default();
    let mut out = ListResult::default();
    match lightsail::list(&aws.sdk, &region).await {
        Ok(v) => out.apps.extend(v),
        Err(e) => out.warnings.push(format!("Lightsail: {e:#}")),
    }
    match ec2::list(&aws.sdk, &region).await {
        Ok(v) => out.apps.extend(v),
        Err(e) => out.warnings.push(format!("EC2: {e:#}")),
    }
    out
}

/// Build, envio da imagem e publicação no destino configurado.
pub async fn deploy(aws: &Aws, cfg: &Config, progress: &Progress) -> Result<Outcome> {
    cfg.validate()?;
    // com uma imagem já pronta não há build, então não precisa de pasta nem de Dockerfile
    if cfg.image.is_none() {
        if !std::path::Path::new(&cfg.context).is_dir() {
            anyhow::bail!("context não encontrado: {}", cfg.context);
        }
        if !std::path::Path::new(&cfg.dockerfile).is_file() {
            anyhow::bail!("Dockerfile não encontrado: {}", cfg.dockerfile);
        }
    }
    if !docker::available().await {
        anyhow::bail!("docker não está disponível");
    }
    let account = aws.account().await?; // falha cedo se as credenciais estiverem erradas
    let url = match cfg.target {
        // Ordem das etapas na tela: Build, Service, Upload, Publish, WaitLive. A instância é preparada
        // antes do envio da imagem para que as etapas apareçam na ordem em que acontecem.
        Target::Ec2 => {
            let image = ecr::build(&aws.sdk, cfg, &account, progress).await?;
            let instance = ec2::prepare(&aws.sdk, cfg, progress).await?;
            ecr::push(&aws.sdk, &image, progress).await?;
            ec2::publish(&aws.sdk, cfg, &instance, &image, progress).await?
        }
        // Lightsail usa o registry do próprio serviço, sem ECR
        Target::Lightsail => lightsail::deploy(&aws.sdk, cfg, progress).await?,
    };
    let url = (!url.is_empty()).then_some(url);
    progress.emit(Event::Done { url: url.clone() });
    Ok(Outcome { url })
}

/// Remove um app achado na conta (por [`list_apps`]), sem precisar de uma pasta de projeto. Os dados
/// vêm de fora, então passam pela mesma validação de nome e região de um `skyship.toml`.
/// Erro devolvido por [`destroy_app`] quando o app não tem a tag do skyship e não foi liberado apagar.
pub const ERR_UNMANAGED: &str = "unmanaged";

pub async fn destroy_app(
    aws: &Aws,
    name: &str,
    region: &str,
    target: Target,
    allow_unmanaged: bool,
    progress: &Progress,
) -> Result<()> {
    let cfg = Config::new(name.to_string(), region.to_string(), 80, target);
    cfg.validate()?;
    if target == Target::Lightsail && !allow_unmanaged && lightsail::managed(&aws.sdk, &cfg.name).await? == Some(false)
    {
        anyhow::bail!(ERR_UNMANAGED);
    }
    destroy(aws, &cfg, progress).await
}

pub async fn status(aws: &Aws, cfg: &Config) -> Result<Vec<StatusEntry>> {
    cfg.validate()?;
    match cfg.target {
        Target::Ec2 => ec2::status(&aws.sdk, cfg).await,
        Target::Lightsail => lightsail::status(&aws.sdk, cfg).await,
    }
}

/// Remove a infraestrutura de compute (instância ou service). Não pede confirmação:
/// quem chama é responsável por isso.
pub async fn destroy(aws: &Aws, cfg: &Config, progress: &Progress) -> Result<()> {
    cfg.validate()?;
    match cfg.target {
        Target::Ec2 => ec2::destroy(&aws.sdk, cfg, progress).await,
        Target::Lightsail => lightsail::destroy(&aws.sdk, cfg, progress).await,
    }
}

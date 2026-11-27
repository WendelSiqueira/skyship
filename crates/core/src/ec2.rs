use crate::{
    AppSummary, StatusEntry,
    config::{Config, Target},
    ecr::Image,
    progress::{Progress, Stage},
};
use anyhow::{Context, Result, bail};
use aws_sdk_ec2::error::{DisplayErrorContext, ProvideErrorMetadata};
use aws_sdk_ec2::types::{
    Filter, IamInstanceProfileSpecification, InstanceStateName, ResourceType, Tag, TagSpecification,
};
use std::time::Duration;
use tokio::time::sleep;

const TRUST: &str = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}"#;
const USER_DATA: &str = "#!/bin/bash\ndnf install -y docker\nsystemctl enable --now docker\n";

/// As duas políticas gerenciadas da AWS que a role da instância leva (e só elas).
const MANAGED_POLICIES: [&str; 2] = [
    "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore",
    "arn:aws:iam::aws:policy/AmazonEC2ContainerRegistryReadOnly",
];

fn res_name(cfg: &Config) -> String {
    format!("skyship-{}", cfg.name)
}

async fn find_instances(
    ec2: &aws_sdk_ec2::Client,
    cfg: &Config,
) -> Result<Vec<(String, InstanceStateName, Option<String>)>> {
    let out = ec2
        .describe_instances()
        .filters(Filter::builder().name("tag:skyship:name").values(&cfg.name).build())
        .filters(
            Filter::builder()
                .name("instance-state-name")
                .values("pending")
                .values("running")
                .values("stopped")
                .build(),
        )
        .send()
        .await?;
    let mut v = vec![];
    for r in out.reservations() {
        for i in r.instances() {
            if let (Some(id), Some(st)) = (i.instance_id(), i.state().and_then(|s| s.name())) {
                v.push((id.to_string(), st.clone(), i.public_ip_address().map(String::from)));
            }
        }
    }
    Ok(v)
}

async fn ensure_role(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<String> {
    let iam = aws_sdk_iam::Client::new(sdk);
    let name = res_name(cfg);
    if iam.get_role().role_name(&name).send().await.is_err() {
        p.log(format!("criando role IAM {name}"));
        iam.create_role()
            .role_name(&name)
            .assume_role_policy_document(TRUST)
            .send()
            .await?;
        for policy in MANAGED_POLICIES {
            iam.attach_role_policy()
                .role_name(&name)
                .policy_arn(policy)
                .send()
                .await?;
        }
    }
    if iam
        .get_instance_profile()
        .instance_profile_name(&name)
        .send()
        .await
        .is_err()
    {
        iam.create_instance_profile()
            .instance_profile_name(&name)
            .send()
            .await?;
        iam.add_role_to_instance_profile()
            .instance_profile_name(&name)
            .role_name(&name)
            .send()
            .await?;
        // o IAM leva alguns segundos para propagar o profile
        sleep(Duration::from_secs(10)).await;
    }
    Ok(name)
}

async fn ensure_sg(ec2: &aws_sdk_ec2::Client, cfg: &Config, p: &Progress) -> Result<String> {
    let name = res_name(cfg);
    let found = ec2
        .describe_security_groups()
        .filters(Filter::builder().name("group-name").values(&name).build())
        .send()
        .await?;
    if let Some(id) = found.security_groups().first().and_then(|g| g.group_id()) {
        return Ok(id.to_string());
    }
    p.log(format!("criando security group {name} (porta 80 aberta)"));
    let id = ec2
        .create_security_group()
        .group_name(&name)
        .description(format!("skyship {}", cfg.name))
        .send()
        .await?
        .group_id()
        .context("sem group id")?
        .to_string();
    ec2.authorize_security_group_ingress()
        .group_id(&id)
        .ip_protocol("tcp")
        .from_port(80)
        .to_port(80)
        .cidr_ip("0.0.0.0/0")
        .send()
        .await?;
    Ok(id)
}

async fn ensure_instance(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<String> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    if let Some((id, state, _)) = find_instances(&ec2, cfg).await?.into_iter().next() {
        if state == InstanceStateName::Stopped {
            p.log(format!("iniciando instância parada {id}"));
            ec2.start_instances().instance_ids(&id).send().await?;
        }
        return Ok(id);
    }

    let profile = ensure_role(sdk, cfg, p).await?;
    let sg = ensure_sg(&ec2, cfg, p).await?;
    let ssm = aws_sdk_ssm::Client::new(sdk);
    let ami = ssm
        .get_parameter()
        .name("/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-x86_64")
        .send()
        .await?
        .parameter()
        .and_then(|p| p.value())
        .context("AMI não encontrada")?
        .to_string();

    p.log(format!("lançando EC2 {} ({})", cfg.ec2.instance_type, ami));
    use base64::Engine;
    let out = ec2
        .run_instances()
        .image_id(ami)
        .instance_type(cfg.ec2.instance_type.as_str().into())
        .min_count(1)
        .max_count(1)
        .security_group_ids(sg)
        .iam_instance_profile(IamInstanceProfileSpecification::builder().name(profile).build())
        .user_data(base64::engine::general_purpose::STANDARD.encode(USER_DATA))
        .tag_specifications(
            TagSpecification::builder()
                .resource_type(ResourceType::Instance)
                .tags(Tag::builder().key("skyship:name").value(&cfg.name).build())
                .tags(Tag::builder().key("Name").value(res_name(cfg)).build())
                .build(),
        )
        .send()
        .await?;
    Ok(out
        .instances()
        .first()
        .and_then(|i| i.instance_id())
        .context("sem instance id")?
        .to_string())
}

async fn wait_running(ec2: &aws_sdk_ec2::Client, id: &str) -> Result<String> {
    for _ in 0..60 {
        let out = ec2.describe_instances().instance_ids(id).send().await?;
        let inst = out.reservations().first().and_then(|r| r.instances().first());
        if let Some(i) = inst
            && i.state().and_then(|s| s.name()) == Some(&InstanceStateName::Running)
            && let Some(ip) = i.public_ip_address()
        {
            return Ok(ip.to_string());
        }
        sleep(Duration::from_secs(5)).await;
    }
    bail!("instância {id} não ficou running a tempo")
}

async fn wait_ssm(ssm: &aws_sdk_ssm::Client, id: &str, p: &Progress) -> Result<()> {
    p.log("esperando o agente SSM da instância (pode levar ~1-2 min)");
    for _ in 0..60 {
        let out = ssm
            .describe_instance_information()
            .filters(
                aws_sdk_ssm::types::InstanceInformationStringFilter::builder()
                    .key("InstanceIds")
                    .values(id)
                    .build()?,
            )
            .send()
            .await?;
        if !out.instance_information_list().is_empty() {
            return Ok(());
        }
        sleep(Duration::from_secs(5)).await;
    }
    bail!("agente SSM não ficou online em {id}")
}

/// Instância pronta para receber o app.
pub struct Instance {
    pub id: String,
    pub ip: String,
}

/// Resume os status checks (do sistema e da instância) em uma palavra: `ok` só quando os dois passaram.
pub fn summarize_checks(system: &str, instance: &str) -> &'static str {
    let both = [system, instance];
    if both.contains(&"impaired") {
        "impaired"
    } else if both.contains(&"initializing") {
        "initializing"
    } else if both.iter().all(|s| *s == "ok") {
        "ok"
    } else {
        "insufficient-data"
    }
}

/// Status checks da instância, como na coluna "Status check" do console da AWS. `None` se não foi possível
/// ler (por exemplo, falta a permissão `ec2:DescribeInstanceStatus`).
async fn instance_checks(ec2: &aws_sdk_ec2::Client, ids: &[String]) -> Vec<(String, String)> {
    let Ok(out) = ec2
        .describe_instance_status()
        .set_instance_ids(Some(ids.to_vec()))
        .include_all_instances(true)
        .send()
        .await
    else {
        return vec![];
    };
    out.instance_statuses()
        .iter()
        .filter_map(|s| {
            let id = s.instance_id()?.to_string();
            let system = s
                .system_status()
                .and_then(|x| x.status())
                .map(|x| x.as_str())
                .unwrap_or("");
            let inst = s
                .instance_status()
                .and_then(|x| x.status())
                .map(|x| x.as_str())
                .unwrap_or("");
            Some((id, summarize_checks(system, inst).to_string()))
        })
        .collect()
}

/// Etapa "Service": cria ou acorda a instância e espera o agente SSM. Devolve o id e o IP público.
pub async fn prepare(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<Instance> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    let ssm = aws_sdk_ssm::Client::new(sdk);

    p.start(Stage::Service);
    let id = ensure_instance(sdk, cfg, p).await?;
    let ip = wait_running(&ec2, &id).await?;
    wait_ssm(&ssm, &id, p).await?;
    if let Some((_, checks)) = instance_checks(&ec2, std::slice::from_ref(&id)).await.first() {
        p.log(format!("status checks da instância: {checks}"));
    }
    p.done(Stage::Service);
    Ok(Instance { id, ip })
}

/// Etapas "Publish" e "WaitLive": baixa a imagem na instância, troca o container e espera o app responder.
/// Devolve a URL pública do app (`http://`: a EC2 não tem HTTPS).
pub async fn publish(
    sdk: &aws_config::SdkConfig,
    cfg: &Config,
    instance: &Instance,
    image: &Image,
    p: &Progress,
) -> Result<String> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    let ssm = aws_sdk_ssm::Client::new(sdk);
    let Instance { id, ip } = instance;

    let script = vec![
        "until command -v docker >/dev/null 2>&1 && systemctl is-active --quiet docker; do sleep 3; done".to_string(),
        format!(
            "aws ecr get-login-password --region {} | docker login --username AWS --password-stdin {}",
            cfg.region, image.registry
        ),
        format!("docker pull {}", image.uri),
        "docker rm -f app || true".to_string(),
        format!(
            "docker run -d --name app --restart unless-stopped -p 80:{} {}",
            cfg.port, image.uri
        ),
        "docker image prune -af --filter until=24h || true".to_string(),
    ];

    p.start(Stage::Publish);
    p.log(format!(
        "executando deploy na instância {id} com a imagem {}",
        image.uri
    ));
    let cmd_id = ssm
        .send_command()
        .instance_ids(id)
        .document_name("AWS-RunShellScript")
        .parameters("commands", script)
        .send()
        .await?
        .command()
        .and_then(|c| c.command_id())
        .context("sem command id")?
        .to_string();

    // até 15 minutos: puxar uma imagem grande pode demorar
    let mut finished = false;
    for _ in 0..300 {
        sleep(Duration::from_secs(3)).await;
        let Ok(inv) = ssm
            .get_command_invocation()
            .command_id(&cmd_id)
            .instance_id(id)
            .send()
            .await
        else {
            continue; // a invocação pode demorar a existir
        };
        match inv.status().map(|s| s.as_str()) {
            Some("Success") => {
                finished = true;
                break;
            }
            Some("Failed") | Some("Cancelled") | Some("TimedOut") => {
                bail!(
                    "deploy falhou ({}):\n{}",
                    inv.status().unwrap().as_str(),
                    inv.standard_error_content().unwrap_or("")
                );
            }
            _ => {}
        }
    }
    if !finished {
        bail!("deploy não terminou a tempo");
    }
    p.done(Stage::Publish);

    // O container já foi iniciado; agora espera a porta 80 responder e mostra os status checks da instância.
    p.start(Stage::WaitLive);
    let url = format!("http://{ip}");
    let mut last_log = Duration::ZERO;
    let started = std::time::Instant::now();
    for _ in 0..60 {
        if http_responds(ip, 80).await {
            let checks = instance_checks(&ec2, std::slice::from_ref(id)).await;
            let state = checks.first().map(|c| c.1.as_str()).unwrap_or("?");
            p.log(format!(
                "o app respondeu em {url} (status checks da instância: {state})"
            ));
            p.done(Stage::WaitLive);
            return Ok(url);
        }
        let elapsed = started.elapsed();
        if elapsed - last_log >= Duration::from_secs(15) {
            last_log = elapsed;
            let checks = instance_checks(&ec2, std::slice::from_ref(id)).await;
            let state = checks.first().map(|c| c.1.as_str()).unwrap_or("?");
            p.log(format!(
                "esperando o app responder em {url} (status checks da instância: {state})"
            ));
        }
        sleep(Duration::from_secs(5)).await;
    }
    bail!(
        "o container subiu, mas a porta 80 de {ip} não respondeu em 5 minutos; veja os status checks da instância no console da AWS"
    )
}

/// A porta responde HTTP? Qualquer resposta HTTP serve, mesmo com erro: o que importa é o app estar de pé.
async fn http_responds(host: &str, port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let probe = async {
        let mut s = tokio::net::TcpStream::connect((host, port)).await.ok()?;
        s.write_all(format!("GET / HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .ok()?;
        let mut first = [0u8; 7];
        s.read_exact(&mut first).await.ok()?;
        Some(first.starts_with(b"HTTP/1."))
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(5), probe).await,
        Ok(Some(true))
    )
}

pub async fn status(sdk: &aws_config::SdkConfig, cfg: &Config) -> Result<Vec<StatusEntry>> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    Ok(find_instances(&ec2, cfg)
        .await?
        .into_iter()
        .map(|(id, state, ip)| StatusEntry {
            id,
            state: state.as_str().to_string(),
            url: ip.map(|i| format!("http://{i}")),
        })
        .collect())
}

/// Instâncias da região que o skyship criou (marcadas com a tag `skyship:name`).
pub async fn list(sdk: &aws_config::SdkConfig, region: &str) -> Result<Vec<AppSummary>> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    let out = ec2
        .describe_instances()
        .filters(Filter::builder().name("tag-key").values("skyship:name").build())
        .filters(
            Filter::builder()
                .name("instance-state-name")
                .values("pending")
                .values("running")
                .values("stopped")
                .build(),
        )
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("{}", aws_sdk_ec2::error::DisplayErrorContext(&e)))?;
    let mut apps: Vec<AppSummary> = vec![];
    for r in out.reservations() {
        for i in r.instances() {
            let name = i
                .tags()
                .iter()
                .find(|t| t.key() == Some("skyship:name"))
                .and_then(|t| t.value())
                .unwrap_or("")
                .to_string();
            apps.push(AppSummary {
                name,
                target: Target::Ec2,
                state: i
                    .state()
                    .and_then(|s| s.name())
                    .map(|n| n.as_str())
                    .unwrap_or("?")
                    .to_string(),
                url: i.public_ip_address().map(|ip| format!("http://{ip}")),
                region: region.to_string(),
                size: i.instance_type().map(|t| t.as_str().to_string()),
                managed: true, // só entram na lista as instâncias com a tag do skyship
                checks: None,
            });
        }
    }
    // status checks (a coluna "Status check" do console); sem a permissão, ficam em branco
    let ids: Vec<String> = ids_of_instances(&out);
    let checks = instance_checks(&ec2, &ids).await;
    for (app, id) in apps.iter_mut().zip(ids.iter()) {
        app.checks = checks.iter().find(|(i, _)| i == id).map(|(_, c)| c.clone());
    }
    Ok(apps)
}

fn ids_of_instances(out: &aws_sdk_ec2::operation::describe_instances::DescribeInstancesOutput) -> Vec<String> {
    out.reservations()
        .iter()
        .flat_map(|r| r.instances())
        .filter_map(|i| i.instance_id().map(String::from))
        .collect()
}

/// Remove o app: encerra a instância e apaga o que o skyship criou para ele (security group, repositório do
/// ECR, role e instance profile do IAM). A limpeza é "melhor esforço": se algo não puder ser apagado, o log diz
/// o quê e o resto segue. Também serve para limpar o que sobrou de uma remoção anterior (sem instância).
pub async fn destroy(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<()> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    let ids: Vec<String> = find_instances(&ec2, cfg).await?.into_iter().map(|i| i.0).collect();
    if ids.is_empty() {
        p.log("nenhuma instância para remover");
    } else {
        p.log(format!("terminando {}", ids.join(", ")));
        ec2.terminate_instances()
            .set_instance_ids(Some(ids.clone()))
            .send()
            .await?;
        if let Err(e) = wait_terminated(&ec2, &ids).await {
            p.log(format!("{e:#}"));
        }
    }
    delete_security_group(&ec2, cfg, p).await;
    delete_ecr_repository(sdk, cfg, p).await;
    delete_role_and_profile(sdk, cfg, p).await;
    Ok(())
}

/// Espera as instâncias chegarem a `terminated`: o security group só pode ser apagado depois.
async fn wait_terminated(ec2: &aws_sdk_ec2::Client, ids: &[String]) -> Result<()> {
    for _ in 0..120 {
        let out = ec2
            .describe_instances()
            .set_instance_ids(Some(ids.to_vec()))
            .send()
            .await?;
        let all_gone = out
            .reservations()
            .iter()
            .flat_map(|r| r.instances())
            .all(|i| i.state().and_then(|s| s.name()) == Some(&InstanceStateName::Terminated));
        if all_gone {
            return Ok(());
        }
        sleep(Duration::from_secs(3)).await;
    }
    bail!("a instância não terminou de encerrar em 6 minutos; o que depende dela pode não ser apagado agora")
}

async fn delete_security_group(ec2: &aws_sdk_ec2::Client, cfg: &Config, p: &Progress) {
    let name = res_name(cfg);
    let Ok(found) = ec2
        .describe_security_groups()
        .filters(Filter::builder().name("group-name").values(&name).build())
        .send()
        .await
    else {
        p.log(format!("não consegui procurar o security group {name}; mantido"));
        return;
    };
    let Some(group) = found.security_groups().first() else {
        return;
    };
    // só apaga o que o skyship criou: a descrição é a que ensure_sg grava
    if group.description() != Some(format!("skyship {}", cfg.name).as_str()) {
        p.log(format!(
            "o security group {name} não foi criado pelo skyship (descrição diferente); mantido"
        ));
        return;
    }
    let Some(id) = group.group_id() else { return };
    // logo depois de encerrar a instância a AWS ainda pode dizer que o grupo está em uso
    for _ in 0..24 {
        match ec2.delete_security_group().group_id(id).send().await {
            Ok(_) => {
                p.log(format!("security group {name} removido"));
                return;
            }
            Err(e) if e.code() == Some("DependencyViolation") => sleep(Duration::from_secs(5)).await,
            Err(e) => {
                p.log(format!(
                    "não consegui remover o security group {name}: {}",
                    DisplayErrorContext(&e)
                ));
                return;
            }
        }
    }
    p.log(format!("o security group {name} continua em uso; remova à mão depois"));
}

async fn delete_ecr_repository(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) {
    let ecr = aws_sdk_ecr::Client::new(sdk);
    let repo = format!("skyship/{}", cfg.name);
    match ecr.delete_repository().repository_name(&repo).force(true).send().await {
        Ok(_) => p.log(format!("repositório ECR {repo} removido, com as imagens")),
        Err(e) if e.code() == Some("RepositoryNotFoundException") => {}
        Err(e) => p.log(format!(
            "não consegui remover o repositório ECR {repo}: {}",
            aws_sdk_ecr::error::DisplayErrorContext(&e)
        )),
    }
}

/// Desfaz a codificação de URL do documento de política que o IAM devolve (`%7B%22Version%22...`).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Dois documentos JSON são iguais, ignorando espaços e a ordem das chaves?
fn same_json(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Alguma instância deste app existe em outra região? A role do IAM é global (vale para todas as regiões).
async fn used_in_other_region(sdk: &aws_config::SdkConfig, cfg: &Config) -> Option<String> {
    let here = sdk.region().map(|r| r.to_string()).unwrap_or_default();
    let mut set = tokio::task::JoinSet::new();
    for r in crate::regions::REGIONS.iter().filter(|r| r.code != here) {
        let conf = sdk
            .clone()
            .into_builder()
            .region(aws_config::Region::new(r.code))
            .build();
        let name = cfg.name.clone();
        set.spawn(async move {
            let ec2 = aws_sdk_ec2::Client::new(&conf);
            let out = ec2
                .describe_instances()
                .filters(Filter::builder().name("tag:skyship:name").values(&name).build())
                .filters(
                    Filter::builder()
                        .name("instance-state-name")
                        .values("pending")
                        .values("running")
                        .values("stopping")
                        .values("stopped")
                        .build(),
                )
                .send()
                .await
                .ok()?; // região não habilitada ou sem acesso: ignora
            out.reservations()
                .iter()
                .any(|x| !x.instances().is_empty())
                .then(|| r.code.to_string())
        });
    }
    while let Some(res) = set.join_next().await {
        if let Ok(Some(region)) = res {
            set.abort_all();
            return Some(region);
        }
    }
    None
}

/// Apaga a role e o instance profile `skyship-<nome>`, se forem do skyship: mesma política de confiança, só
/// as duas políticas gerenciadas, e nenhuma instância do app em outra região (a role é global).
async fn delete_role_and_profile(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) {
    let iam = aws_sdk_iam::Client::new(sdk);
    let name = res_name(cfg);
    let Ok(role) = iam.get_role().role_name(&name).send().await else {
        return; // não existe (ou sem permissão para ler): nada a apagar
    };
    let trust = percent_decode(
        role.role()
            .and_then(|r| r.assume_role_policy_document())
            .unwrap_or_default(),
    );
    if !same_json(&trust, TRUST) {
        p.log(format!(
            "a role {name} não foi criada pelo skyship (política de confiança diferente); mantida"
        ));
        return;
    }
    let attached = match iam.list_attached_role_policies().role_name(&name).send().await {
        Ok(o) => o
            .attached_policies()
            .iter()
            .filter_map(|a| a.policy_arn().map(String::from))
            .collect::<Vec<_>>(),
        Err(e) => {
            p.log(format!(
                "não consegui ler as políticas da role {name}; mantida: {}",
                aws_sdk_iam::error::DisplayErrorContext(&e)
            ));
            return;
        }
    };
    if attached.iter().any(|a| !MANAGED_POLICIES.contains(&a.as_str())) {
        p.log(format!(
            "a role {name} tem outras políticas além das do skyship; mantida"
        ));
        return;
    }
    if let Some(region) = used_in_other_region(sdk, cfg).await {
        p.log(format!(
            "a role {name} também é usada por uma instância em {region}; mantida"
        ));
        return;
    }
    // o profile primeiro: ele guarda a role
    if iam
        .get_instance_profile()
        .instance_profile_name(&name)
        .send()
        .await
        .is_ok()
    {
        let _ = iam
            .remove_role_from_instance_profile()
            .instance_profile_name(&name)
            .role_name(&name)
            .send()
            .await;
        match iam.delete_instance_profile().instance_profile_name(&name).send().await {
            Ok(_) => p.log(format!("instance profile {name} removido")),
            Err(e) => p.log(format!(
                "não consegui remover o instance profile {name}: {}",
                aws_sdk_iam::error::DisplayErrorContext(&e)
            )),
        }
    }
    for policy in &attached {
        let _ = iam
            .detach_role_policy()
            .role_name(&name)
            .policy_arn(policy)
            .send()
            .await;
    }
    match iam.delete_role().role_name(&name).send().await {
        Ok(_) => p.log(format!("role IAM {name} removida")),
        Err(e) => p.log(format!(
            "não consegui remover a role {name}: {}",
            aws_sdk_iam::error::DisplayErrorContext(&e)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{TRUST, http_responds, percent_decode, same_json, summarize_checks};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn checks_are_ok_only_when_both_pass() {
        assert_eq!(summarize_checks("ok", "ok"), "ok");
        assert_eq!(summarize_checks("initializing", "initializing"), "initializing");
        assert_eq!(summarize_checks("ok", "initializing"), "initializing");
        assert_eq!(summarize_checks("ok", "impaired"), "impaired");
        assert_eq!(summarize_checks("initializing", "impaired"), "impaired");
        assert_eq!(summarize_checks("", ""), "insufficient-data");
        assert_eq!(summarize_checks("not-applicable", "ok"), "insufficient-data");
    }

    #[test]
    fn decodes_the_policy_document_the_way_iam_returns_it() {
        let encoded = "%7B%22Version%22%3A%222012-10-17%22%2C%22Statement%22%3A%5B%7B%22Effect%22%3A%22Allow%22%2C%22Principal%22%3A%7B%22Service%22%3A%22ec2.amazonaws.com%22%7D%2C%22Action%22%3A%22sts%3AAssumeRole%22%7D%5D%7D";
        assert!(
            same_json(&percent_decode(encoded), TRUST),
            "a role criada pelo skyship precisa ser reconhecida"
        );
        assert_eq!(percent_decode("a%2Fb+c%"), "a/b c%");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn only_the_exact_trust_policy_counts_as_skyships() {
        // outra entidade pode assumir a role: não é a do skyship
        let other = TRUST.replace("ec2.amazonaws.com", "lambda.amazonaws.com");
        assert!(!same_json(&other, TRUST));
        assert!(!same_json("não é json", TRUST));
        // espaços e a ordem das chaves não importam
        let reordered = r#"{ "Statement": [ { "Action": "sts:AssumeRole", "Principal": { "Service": "ec2.amazonaws.com" }, "Effect": "Allow" } ], "Version": "2012-10-17" }"#;
        assert!(same_json(reordered, TRUST));
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn any_http_answer_counts_as_alive_even_an_error() {
        rt().block_on(async {
            for reply in ["HTTP/1.1 200 OK\r\n\r\n", "HTTP/1.0 503 Service Unavailable\r\n\r\n"] {
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = l.local_addr().unwrap().port();
                tokio::spawn(async move {
                    let (mut s, _) = l.accept().await.unwrap();
                    // lê o pedido antes de responder, como um servidor de verdade: no Windows, fechar
                    // o socket com dados não lidos manda RST e o cliente perde a resposta
                    let mut req = Vec::new();
                    let mut buf = [0u8; 512];
                    while !req.ends_with(b"\r\n\r\n") {
                        let n = s.read(&mut buf).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        req.extend_from_slice(&buf[..n]);
                    }
                    s.write_all(reply.as_bytes()).await.unwrap();
                    s.shutdown().await.unwrap();
                });
                assert!(http_responds("127.0.0.1", port).await, "{reply}");
            }
        });
    }

    #[test]
    fn closed_port_or_non_http_is_not_alive() {
        rt().block_on(async {
            // porta sem ninguém ouvindo
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let closed = l.local_addr().unwrap().port();
            drop(l);
            assert!(!http_responds("127.0.0.1", closed).await);
            // alguém ouve, mas não fala HTTP
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = l.local_addr().unwrap().port();
            tokio::spawn(async move {
                let (mut s, _) = l.accept().await.unwrap();
                s.write_all(b"SSH-2.0-OpenSSH\r\n").await.unwrap();
            });
            assert!(!http_responds("127.0.0.1", port).await);
        });
    }
}

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use skyship_core::{
    Aws,
    config::{self, Config, Target},
    docker, login,
    progress::{Event, Progress, Stage},
};
use std::io::{BufRead, Write};

#[derive(Parser)]
#[command(
    name = "skyship",
    version,
    about = "Docker → AWS (EC2 / Lightsail) direto da sua máquina"
)]
struct Cli {
    /// Perfil AWS (de ~/.aws/credentials); também lê AWS_PROFILE
    #[arg(long, global = true, env = "AWS_PROFILE")]
    profile: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Cria o skyship.toml no diretório atual
    Init {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "us-east-1")]
        region: String,
        #[arg(long, default_value_t = 8080)]
        port: u16,
        #[arg(long, default_value = "ec2")]
        target: Target,
    },
    /// Guarda suas chaves AWS em um perfil local, validando antes de gravar
    Login {
        #[arg(long, default_value = "us-east-1")]
        region: String,
    },
    /// Lista as regiões aceitas
    Regions,
    /// Imprime a política IAM necessária para o destino
    Policy {
        #[arg(long, default_value = "lightsail")]
        target: Target,
    },
    /// Verifica docker e credenciais AWS
    Doctor,
    /// Build, envio da imagem e deploy no destino configurado
    Deploy,
    /// Mostra o estado do deploy
    Status,
    /// Remove a infraestrutura de compute (instância ou service)
    Destroy {
        #[arg(long)]
        yes: bool,
    },
}

fn stage_label(s: Stage) -> &'static str {
    match s {
        Stage::Build => "construindo a imagem",
        Stage::Service => "preparando o serviço",
        Stage::Upload => "enviando a imagem",
        Stage::Publish => "publicando a nova versão",
        Stage::WaitLive => "esperando o app responder",
    }
}

/// Mostra os eventos do core no terminal.
fn terminal_progress() -> Progress {
    Progress::new(|e| match e {
        Event::StageStarted { stage } => println!("→ {}", stage_label(stage)),
        Event::StageDone { .. } => {}
        Event::Log { line } => println!("  {line}"),
        Event::Done { url } => match url {
            Some(u) => println!("✔ no ar: {u}"),
            None => println!("✔ concluído"),
        },
        Event::Failed { message } => eprintln!("✘ {message}"),
        Event::Cancelled => eprintln!("✘ cancelado"),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let profile = cli.profile;
    match cli.cmd {
        Cmd::Init {
            name,
            region,
            port,
            target,
        } => {
            let name = match name {
                Some(n) => n,
                None => std::env::current_dir()?
                    .file_name()
                    .map(|s| s.to_string_lossy().to_lowercase().replace(['_', ' '], "-"))
                    .unwrap_or_else(|| "app".into()),
            };
            Config::new(name, region, port, target).save()?;
            println!("✔ {} criado", config::FILE);
        }
        Cmd::Login { region } => {
            let profile = profile.as_deref().unwrap_or("skyship");
            println!("Perfil: {profile}");
            println!("Crie as chaves em IAM → Users → (seu usuário) → Security credentials → Create access key.");
            print!("Access key ID: ");
            std::io::stdout().flush()?;
            let mut key_id = String::new();
            std::io::stdin().lock().read_line(&mut key_id)?;
            let secret = rpassword::prompt_password("Secret access key (não aparece enquanto digita): ")?;

            let id = login::verify(&key_id, &secret, &region).await?;
            println!("✔ chaves válidas: conta {} ({})", id.account, id.arn);
            let path = login::save_profile(profile, &key_id, &secret)?;
            let modo = if cfg!(unix) { " (permissão 600)" } else { "" };
            println!("✔ perfil [{profile}] gravado em {}{modo}", path.display());
            println!("Use: skyship --profile {profile} deploy   (ou AWS_PROFILE={profile})");
        }
        Cmd::Regions => {
            println!("{:<16} {:<18} Lightsail (não verificado)", "REGIÃO", "LOCAL");
            for r in skyship_core::regions::REGIONS {
                println!(
                    "{:<16} {:<18} {}",
                    r.code,
                    r.name,
                    if r.lightsail { "sim" } else { "-" }
                );
            }
        }
        Cmd::Policy { target } => println!("{}", login::policy(target)),
        Cmd::Doctor => {
            println!(
                "docker: {}",
                if docker::available().await {
                    "ok"
                } else {
                    "NÃO encontrado/rodando"
                }
            );
            let region = Config::load().map(|c| c.region).unwrap_or_else(|_| "us-east-1".into());
            let aws = Aws::connect(&region, profile.as_deref()).await;
            match aws.account().await {
                Ok(a) => println!("aws: conta {a} (região {region})"),
                Err(e) => println!("aws: {e}"),
            }
        }
        Cmd::Deploy => {
            let cfg = Config::load()?;
            let aws = Aws::connect(&cfg.region, profile.as_deref()).await;
            skyship_core::deploy(&aws, &cfg, &terminal_progress()).await?;
        }
        Cmd::Status => {
            let cfg = Config::load()?;
            let aws = Aws::connect(&cfg.region, profile.as_deref()).await;
            let entries = skyship_core::status(&aws, &cfg).await?;
            if entries.is_empty() {
                println!("nada rodando para {}", cfg.name);
            }
            for e in entries {
                println!("{}  {}  {}", e.id, e.state, e.url.unwrap_or_default());
            }
        }
        Cmd::Destroy { yes } => {
            let cfg = Config::load()?;
            if !yes {
                bail!(
                    "isso apaga a infraestrutura de {} ({:?}); repita com --yes",
                    cfg.name,
                    cfg.target
                );
            }
            let aws = Aws::connect(&cfg.region, profile.as_deref()).await;
            skyship_core::destroy(&aws, &cfg, &terminal_progress()).await?;
        }
    }
    Ok(())
}

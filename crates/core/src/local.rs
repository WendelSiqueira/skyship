//! Lê o que existe no Docker deste computador (`docker image ls`, `docker ps`) para sugerir o que
//! publicar. Só metadados: variáveis de ambiente, volumes e segredos dos containers NÃO são lidos.
//!
//! Quem chama é responsável por ter a permissão da pessoa para isto (a GUI confere nas configurações).

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;

const COMPOSE_DIR_LABEL: &str = "com.docker.compose.project.working_dir";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalImage {
    /// `repo:tag` da primeira tag da imagem.
    pub reference: String,
    pub tags: Vec<String>,
    /// Id curto (12 caracteres).
    pub id: String,
    pub size_bytes: u64,
    pub created: String,
    pub os: String,
    pub arch: String,
    /// Portas TCP do `EXPOSE`.
    pub exposed_ports: Vec<u16>,
    /// Nomes dos containers em execução que usam esta imagem.
    pub running: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalContainer {
    pub name: String,
    /// Imagem como o container foi criado (`repo:tag`).
    pub image: String,
    pub image_id: String,
    /// Pasta do projeto, quando o container veio do Docker Compose.
    pub project_dir: Option<String>,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct LocalDocker {
    pub images: Vec<LocalImage>,
    pub containers: Vec<LocalContainer>,
}

async fn docker_out(args: &[&str]) -> Result<String> {
    let fut = tokio::process::Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(20), fut)
        .await
        .map_err(|_| anyhow::anyhow!("o docker demorou demais para responder"))?
        .context("falha ao executar docker (ele está instalado e rodando?)")?;
    if !out.status.success() {
        bail!(
            "docker {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Lista imagens (com tag) e containers em execução.
pub async fn list() -> Result<LocalDocker> {
    let image_ids = docker_out(&["image", "ls", "--no-trunc", "-q"]).await?;
    let mut ids: Vec<&str> = image_ids.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    ids.sort_unstable();
    ids.dedup();
    let images_json = if ids.is_empty() {
        "[]".to_string()
    } else {
        let mut args = vec!["image", "inspect"];
        args.extend(&ids);
        docker_out(&args).await?
    };

    let cont_ids = docker_out(&["ps", "-q", "--no-trunc"]).await?;
    let cids: Vec<&str> = cont_ids.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let containers_json = if cids.is_empty() {
        "[]".to_string()
    } else {
        let mut args = vec!["container", "inspect"];
        args.extend(&cids);
        docker_out(&args).await?
    };
    parse(&images_json, &containers_json)
}

fn short_id(id: &str) -> String {
    id.strip_prefix("sha256:").unwrap_or(id).chars().take(12).collect()
}

fn ports_of(config: &Value) -> Vec<u16> {
    let mut v: Vec<u16> = config
        .get("ExposedPorts")
        .and_then(Value::as_object)
        .map(|m| {
            m.keys()
                .filter_map(|k| {
                    let (port, proto) = k.split_once('/').unwrap_or((k, "tcp"));
                    (proto == "tcp").then(|| port.parse::<u16>().ok()).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_unstable();
    v.dedup();
    v
}

/// Interpreta a saída de `docker image inspect` e `docker container inspect` (arrays JSON).
pub fn parse(images_json: &str, containers_json: &str) -> Result<LocalDocker> {
    let imgs: Vec<Value> = serde_json::from_str(images_json).context("resposta do docker em formato inesperado")?;
    let conts: Vec<Value> =
        serde_json::from_str(containers_json).context("resposta do docker em formato inesperado")?;

    let mut containers = vec![];
    for c in &conts {
        let name = c["Name"].as_str().unwrap_or("").trim_start_matches('/').to_string();
        let cfg = &c["Config"];
        let project_dir = cfg["Labels"][COMPOSE_DIR_LABEL]
            .as_str()
            .filter(|p| p.starts_with('/'))
            .map(String::from);
        containers.push(LocalContainer {
            name,
            image: cfg["Image"].as_str().unwrap_or("").to_string(),
            image_id: short_id(c["Image"].as_str().unwrap_or("")),
            project_dir,
            ports: ports_of(cfg),
        });
    }

    let mut images = vec![];
    for i in &imgs {
        let tags: Vec<String> = i["RepoTags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|t| !t.contains("<none>"))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        if tags.is_empty() {
            continue; // sem tag (camada solta): não dá para publicar por nome
        }
        let id = short_id(i["Id"].as_str().unwrap_or(""));
        let running = containers
            .iter()
            .filter(|c| c.image_id == id)
            .map(|c| c.name.clone())
            .collect();
        images.push(LocalImage {
            reference: tags[0].clone(),
            tags,
            id,
            size_bytes: i["Size"].as_u64().unwrap_or(0),
            created: i["Created"].as_str().unwrap_or("").to_string(),
            os: i["Os"].as_str().unwrap_or("").to_string(),
            arch: i["Architecture"].as_str().unwrap_or("").to_string(),
            exposed_ports: ports_of(&i["Config"]),
            running,
        });
    }
    // em execução primeiro, depois as que expõem porta (parecem aplicações), depois por nome
    images.sort_by(|a, b| {
        (!b.running.is_empty(), !b.exposed_ports.is_empty(), &a.reference).cmp(&(
            !a.running.is_empty(),
            !a.exposed_ports.is_empty(),
            &b.reference,
        ))
    });
    Ok(LocalDocker { images, containers })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGES: &str = r#"[
      {"Id":"sha256:aaaaaaaaaaaa1111","RepoTags":["meuapp:1.0","meuapp:latest"],"Created":"2026-11-01T10:00:00Z","Size":123456,
       "Os":"linux","Architecture":"amd64","Config":{"ExposedPorts":{"8080/tcp":{},"53/udp":{}}}},
      {"Id":"sha256:bbbbbbbbbbbb2222","RepoTags":["postgres:18"],"Created":"2026-10-01T10:00:00Z","Size":999,
       "Os":"linux","Architecture":"arm64","Config":{"ExposedPorts":{"5432/tcp":{}}}},
      {"Id":"sha256:cccccccccccc3333","RepoTags":["<none>:<none>"],"Size":5,"Os":"linux","Architecture":"amd64","Config":{}},
      {"Id":"sha256:dddddddddddd4444","RepoTags":null,"Size":5,"Os":"linux","Architecture":"amd64","Config":null}
    ]"#;
    const CONTAINERS: &str = r#"[
      {"Name":"/web","Image":"sha256:aaaaaaaaaaaa1111","Config":{"Image":"meuapp:1.0","ExposedPorts":{"8080/tcp":{}},
        "Labels":{"com.docker.compose.project.working_dir":"/home/pessoa/projetos/meuapp"},"Env":["SENHA=segredo"]}},
      {"Name":"/db","Image":"sha256:bbbbbbbbbbbb2222","Config":{"Image":"postgres:18","Labels":{"com.docker.compose.project.working_dir":"relativo"}}}
    ]"#;

    #[test]
    fn parses_images_and_containers() {
        let d = parse(IMAGES, CONTAINERS).unwrap();
        assert_eq!(d.images.len(), 2, "sem tag não entra");
        let app = d.images.iter().find(|i| i.reference == "meuapp:1.0").unwrap();
        assert_eq!(app.id, "aaaaaaaaaaaa");
        assert_eq!(app.exposed_ports, vec![8080], "só TCP");
        assert_eq!(app.running, vec!["web"]);
        assert_eq!((app.os.as_str(), app.arch.as_str()), ("linux", "amd64"));
        assert_eq!(app.tags.len(), 2);
    }

    #[test]
    fn compose_dir_only_when_absolute_and_env_is_never_read() {
        let d = parse(IMAGES, CONTAINERS).unwrap();
        let web = d.containers.iter().find(|c| c.name == "web").unwrap();
        assert_eq!(web.project_dir.as_deref(), Some("/home/pessoa/projetos/meuapp"));
        let db = d.containers.iter().find(|c| c.name == "db").unwrap();
        assert_eq!(db.project_dir, None);
        // nada do resultado carrega o conteúdo de `Env`
        assert!(!serde_json::to_string(&d).unwrap().contains("segredo"));
    }

    #[test]
    fn running_images_come_first() {
        let d = parse(IMAGES, CONTAINERS).unwrap();
        assert!(d.images[0].running.len() >= d.images[1].running.len());
    }

    /// Precisa do Docker rodando: `cargo test -p skyship-core lists_real_docker -- --ignored --nocapture`.
    /// Imprime só contagens, nunca nomes.
    #[test]
    #[ignore]
    fn lists_real_docker() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let d = rt.block_on(list()).unwrap();
        println!(
            "imagens com tag: {} | com porta: {} | em execução: {} | containers: {} | com pasta do compose: {} | arquiteturas: {:?}",
            d.images.len(),
            d.images.iter().filter(|i| !i.exposed_ports.is_empty()).count(),
            d.images.iter().filter(|i| !i.running.is_empty()).count(),
            d.containers.len(),
            d.containers.iter().filter(|c| c.project_dir.is_some()).count(),
            d.images
                .iter()
                .map(|i| i.arch.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
        );
    }

    #[test]
    fn empty_docker_is_fine() {
        assert_eq!(parse("[]", "[]").unwrap(), LocalDocker::default());
        assert!(parse("não é json", "[]").is_err());
    }
}

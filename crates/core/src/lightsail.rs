use crate::{
    config::{Config, Target},
    docker,
    progress::{Progress, Stage},
    AppSummary, StatusEntry,
};
use anyhow::{bail, Context, Result};
use aws_sdk_lightsail::types::{
    Container, ContainerServiceHealthCheckConfig, ContainerServicePowerName, ContainerServiceProtocol,
    EndpointRequest,
};
use std::time::Duration;
use tokio::time::sleep;

/// Tag que o skyship grava nos serviços que cria ou atualiza.
pub const TAG_KEY: &str = "skyship:name";

fn is_managed(s: &aws_sdk_lightsail::types::ContainerService) -> bool {
    s.tags().iter().any(|t| t.key() == Some(TAG_KEY))
}

/// O serviço existe e foi marcado pelo skyship? `None` = o serviço não existe.
pub async fn managed(sdk: &aws_config::SdkConfig, name: &str) -> Result<Option<bool>> {
    let ls = aws_sdk_lightsail::Client::new(sdk);
    Ok(get(&ls, name).await?.map(|s| is_managed(&s)))
}

fn svc_name(cfg: &Config) -> String {
    cfg.name.clone()
}

async fn get(ls: &aws_sdk_lightsail::Client, name: &str) -> Result<Option<aws_sdk_lightsail::types::ContainerService>> {
    match ls.get_container_services().service_name(name).send().await {
        Ok(o) => Ok(o.container_services().first().cloned()),
        // só "não existe" vira None; credencial inválida, rede etc. precisam aparecer como erro
        Err(e) if e.as_service_error().is_some_and(|s| s.is_not_found_exception()) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("falha ao consultar o Lightsail: {}", aws_sdk_lightsail::error::DisplayErrorContext(&e))),
    }
}

/// Devolve a URL pública do app.
pub async fn deploy(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<String> {
    let ls = aws_sdk_lightsail::Client::new(sdk);
    let name = svc_name(cfg);

    // 1. build local, ou a imagem que já existe (não depende do serviço)
    let tag = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let local = match &cfg.image {
        Some(img) => img.clone(),
        None => format!("skyship-build/{name}:{tag}"),
    };
    p.start(Stage::Build);
    match &cfg.image {
        Some(img) => docker::ensure_platform(img, &cfg.platform, p).await?,
        None => docker::build(&local, &cfg.dockerfile, &cfg.context, &cfg.platform, p).await?,
    }
    p.done(Stage::Build);

    // 2. serviço (precisa existir antes de registrar a imagem)
    p.start(Stage::Service);
    let mut svc = get(&ls, &name).await?;
    if svc.is_none() {
        p.log(format!("criando container service {name} ({} x{})", cfg.lightsail.power, cfg.lightsail.scale));
        ls.create_container_service()
            .service_name(&name)
            .power(ContainerServicePowerName::from(cfg.lightsail.power.as_str()))
            .scale(cfg.lightsail.scale)
            .send()
            .await?;
        svc = get(&ls, &name).await?;
    }
    p.log("esperando o service ficar pronto (a AWS leva alguns minutos para criar o serviço)");
    let mut svc = svc.context("service não encontrado")?;
    let started = std::time::Instant::now();
    let mut last_log = std::time::Duration::ZERO;
    let mut ready = false;
    // até 20 minutos: o serviço começa em PENDING e a AWS o libera quando termina de provisionar
    for _ in 0..240 {
        let state = svc.state().map(|s| s.as_str().to_string()).unwrap_or_default();
        if matches!(state.as_str(), "READY" | "RUNNING" | "DEPLOYING") {
            ready = true;
            break;
        }
        if matches!(state.as_str(), "DISABLED" | "DELETING") {
            bail!("service em estado {state}");
        }
        let elapsed = started.elapsed();
        if elapsed - last_log >= Duration::from_secs(30) {
            last_log = elapsed;
            p.log(format!("ainda esperando: o service está em {} há {} s", if state.is_empty() { "?" } else { &state }, elapsed.as_secs()));
        }
        sleep(Duration::from_secs(5)).await;
        svc = get(&ls, &name).await?.context("service sumiu")?;
    }
    if !ready {
        bail!("o service {name} não ficou pronto em 20 minutos (ainda em {}); ele continua sendo criado na AWS, tente de novo mais tarde",
            svc.state().map(|x| x.as_str()).unwrap_or("?"));
    }
    // Marca o serviço como do skyship (também os criados antes disso). É um extra: sem a permissão
    // `lightsail:TagResource` o deploy segue, só que o app não aparece como do skyship na lista.
    if !is_managed(&svc) {
        let tag = aws_sdk_lightsail::types::Tag::builder().key(TAG_KEY).value(&name).build();
        if let Err(e) = ls.tag_resource().resource_name(&name).tags(tag).send().await {
            p.log(format!(
                "não consegui marcar o service como do skyship ({}); dê a permissão lightsail:TagResource para isso",
                aws_sdk_lightsail::error::DisplayErrorContext(&e)
            ));
        }
    }
    p.done(Stage::Service);

    // 3. push para o registry do próprio Lightsail (sem ECR)
    p.start(Stage::Upload);
    let login = ls
        .create_container_service_registry_login()
        .send()
        .await?
        .registry_login()
        .cloned()
        .context("Lightsail não devolveu login de registry")?;
    let registry = login.registry().context("login sem registry")?.to_string();
    docker::login(&registry, login.username().unwrap_or_default(), login.password().unwrap_or_default(), p).await?;

    // O repositório do Lightsail é sempre `<registry>/sr`: o serviço e o label não entram no caminho
    // (o `lightsailctl` da AWS faz igual; o login só vale para esse repositório). O tag precisa ser único.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let remote = format!("{registry}/sr:{nanos}");
    docker::tag(&local, &remote, p).await?;
    docker::push(&remote, p).await?;
    let digest = docker::repo_digest(&remote, &registry).await?;

    let registered = ls
        .register_container_image()
        .service_name(&name)
        .label("app")
        .digest(digest)
        .send()
        .await?;
    let image = registered
        .container_image()
        .and_then(|i| i.image())
        .context("Lightsail não devolveu o nome da imagem registrada")?
        .to_string();
    p.done(Stage::Upload);

    // 4. deployment
    p.start(Stage::Publish);
    p.log(format!("criando deployment ({image})"));
    ls.create_container_service_deployment()
        .service_name(&name)
        .containers(
            "app",
            Container::builder()
                .image(&image)
                .ports(cfg.port.to_string(), ContainerServiceProtocol::Http)
                .build(),
        )
        .public_endpoint(
            EndpointRequest::builder()
                .container_name("app")
                .container_port(cfg.port as i32)
                .health_check(ContainerServiceHealthCheckConfig::builder().path("/").build())
                .build()?,
        )
        .send()
        .await?;
    p.done(Stage::Publish);

    p.start(Stage::WaitLive);
    for _ in 0..120 {
        sleep(Duration::from_secs(10)).await;
        let s = get(&ls, &name).await?.context("service sumiu")?;
        let dep = s.current_deployment().and_then(|d| d.state()).map(|x| x.as_str().to_string());
        match dep.as_deref() {
            Some("ACTIVE") => {
                p.done(Stage::WaitLive);
                return Ok(s.url().unwrap_or_default().to_string());
            }
            Some("FAILED") => bail!("deployment falhou; veja os logs no console do Lightsail"),
            _ => {}
        }
    }
    bail!("deployment não ficou ativo a tempo")
}

pub async fn status(sdk: &aws_config::SdkConfig, cfg: &Config) -> Result<Vec<StatusEntry>> {
    let ls = aws_sdk_lightsail::Client::new(sdk);
    Ok(match get(&ls, &svc_name(cfg)).await? {
        None => vec![],
        Some(s) => vec![StatusEntry {
            id: s.container_service_name().unwrap_or("").to_string(),
            state: s.state().map(|x| x.as_str()).unwrap_or("?").to_string(),
            url: s.url().map(String::from),
        }],
    })
}

/// Todos os container services da região (inclusive os que não foram criados pelo skyship).
pub async fn list(sdk: &aws_config::SdkConfig, region: &str) -> Result<Vec<AppSummary>> {
    let ls = aws_sdk_lightsail::Client::new(sdk);
    let out = ls
        .get_container_services()
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("{}", aws_sdk_lightsail::error::DisplayErrorContext(&e)))?;
    Ok(out
        .container_services()
        .iter()
        .map(|s| AppSummary {
            name: s.container_service_name().unwrap_or("").to_string(),
            target: Target::Lightsail,
            state: s.state().map(|x| x.as_str()).unwrap_or("?").to_string(),
            url: s.url().filter(|u| !u.is_empty()).map(String::from),
            region: region.to_string(),
            size: s.power().map(|x| x.as_str().to_string()),
            managed: is_managed(s),
        })
        .collect())
}

pub async fn destroy(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<()> {
    let ls = aws_sdk_lightsail::Client::new(sdk);
    let name = svc_name(cfg);
    if get(&ls, &name).await?.is_none() {
        p.log("service não existe");
        return Ok(());
    }
    ls.delete_container_service().service_name(&name).send().await?;
    p.log(format!("service {name} removido"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_lightsail::types::{ContainerService, Tag};

    fn svc(tags: &[(&str, &str)]) -> ContainerService {
        let mut b = ContainerService::builder().container_service_name("x");
        for (k, v) in tags {
            b = b.tags(Tag::builder().key(*k).value(*v).build());
        }
        b.build()
    }

    #[test]
    fn only_the_skyship_tag_marks_a_service() {
        assert!(is_managed(&svc(&[(TAG_KEY, "x")])));
        assert!(is_managed(&svc(&[("outra", "1"), (TAG_KEY, "x")])));
        assert!(!is_managed(&svc(&[])));
        assert!(!is_managed(&svc(&[("skyship", "x"), ("env", "prod")])));
    }
}

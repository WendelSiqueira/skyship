use crate::{config::Config, docker, progress::{Progress, Stage}};
use anyhow::{Context, Result};
use aws_sdk_ecr::Client;
use base64::Engine;

pub struct Image {
    pub uri: String,
    pub registry: String,
}

/// Garante o repositório, faz build local, login e push. Devolve a URI da imagem.
pub async fn build_and_push(sdk: &aws_config::SdkConfig, cfg: &Config, account: &str, p: &Progress) -> Result<Image> {
    let ecr = Client::new(sdk);
    let repo = format!("skyship/{}", cfg.name);

    if ecr.describe_repositories().repository_names(&repo).send().await.is_err() {
        p.log(format!("criando repositório ECR {repo}"));
        ecr.create_repository().repository_name(&repo).send().await?;
    }

    let registry = format!("{account}.dkr.ecr.{}.amazonaws.com", cfg.region);
    let tag = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let uri = format!("{registry}/{repo}:{tag}");

    p.start(Stage::Build);
    match &cfg.image {
        Some(img) => {
            docker::ensure_platform(img, &cfg.platform, p).await?;
            docker::tag(img, &uri, p).await?;
        }
        None => docker::build(&uri, &cfg.dockerfile, &cfg.context, &cfg.platform, p).await?,
    }
    p.done(Stage::Build);

    p.start(Stage::Upload);
    let auth = ecr.get_authorization_token().send().await?;
    let token = auth
        .authorization_data()
        .first()
        .and_then(|d| d.authorization_token())
        .context("ECR não devolveu token")?;
    let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(token)?)?;
    let (user, pass) = decoded.split_once(':').context("token ECR em formato inesperado")?;
    docker::login(&registry, user, pass, p).await?;
    docker::push(&uri, p).await?;
    p.done(Stage::Upload);
    Ok(Image { uri, registry })
}

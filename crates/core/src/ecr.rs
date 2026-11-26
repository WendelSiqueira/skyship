use crate::{
    config::Config,
    docker,
    progress::{Progress, Stage},
};
use anyhow::{Context, Result};
use aws_sdk_ecr::Client;
use base64::Engine;

pub struct Image {
    pub uri: String,
    pub registry: String,
}

/// Quantas imagens o ECR guarda em cada repositório `skyship/<nome>`. As mais antigas são apagadas pela
/// política de ciclo de vida, que a AWS aplica de forma assíncrona (as extras somem em até cerca de um dia).
pub const KEEP_IMAGES: u32 = 3;

/// Política de ciclo de vida do repositório: apaga as imagens além das `KEEP_IMAGES` mais recentes.
fn lifecycle_policy() -> String {
    serde_json::json!({
        "rules": [{
            "rulePriority": 1,
            "description": format!("Mantém só as {KEEP_IMAGES} imagens mais recentes (skyship)"),
            "selection": {
                "tagStatus": "any",
                "countType": "imageCountMoreThan",
                "countNumber": KEEP_IMAGES
            },
            "action": { "type": "expire" }
        }]
    })
    .to_string()
}

/// Etapa "Build": garante o repositório e constrói (ou etiqueta) a imagem localmente. Ainda não envia nada.
pub async fn build(sdk: &aws_config::SdkConfig, cfg: &Config, account: &str, p: &Progress) -> Result<Image> {
    let ecr = Client::new(sdk);
    let repo = format!("skyship/{}", cfg.name);

    if ecr
        .describe_repositories()
        .repository_names(&repo)
        .send()
        .await
        .is_err()
    {
        p.log(format!("criando repositório ECR {repo}"));
        ecr.create_repository().repository_name(&repo).send().await?;
    }
    // Idempotente: vale também para repositórios criados antes da política existir. É um extra: sem a
    // permissão `ecr:PutLifecyclePolicy` o deploy segue, só que as imagens antigas continuam no ECR.
    if let Err(e) = ecr
        .put_lifecycle_policy()
        .repository_name(&repo)
        .lifecycle_policy_text(lifecycle_policy())
        .send()
        .await
    {
        p.log(format!(
            "não consegui configurar a limpeza automática do ECR ({}); dê a permissão ecr:PutLifecyclePolicy para manter só as {KEEP_IMAGES} imagens mais recentes",
            aws_sdk_ecr::error::DisplayErrorContext(&e)
        ));
    } else {
        p.log(format!("ECR {repo}: mantém só as {KEEP_IMAGES} imagens mais recentes"));
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
    Ok(Image { uri, registry })
}

/// Etapa "Upload": login no ECR e push da imagem.
pub async fn push(sdk: &aws_config::SdkConfig, image: &Image, p: &Progress) -> Result<()> {
    let ecr = Client::new(sdk);
    p.start(Stage::Upload);
    let auth = ecr.get_authorization_token().send().await?;
    let token = auth
        .authorization_data()
        .first()
        .and_then(|d| d.authorization_token())
        .context("ECR não devolveu token")?;
    let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(token)?)?;
    let (user, pass) = decoded.split_once(':').context("token ECR em formato inesperado")?;
    docker::login(&image.registry, user, pass, p).await?;
    docker::push(&image.uri, p).await?;
    p.log(format!("imagem enviada ao ECR: {}", image.uri));
    p.done(Stage::Upload);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{KEEP_IMAGES, lifecycle_policy};

    #[test]
    fn lifecycle_keeps_the_latest_images_and_expires_the_rest() {
        let v: serde_json::Value = serde_json::from_str(&lifecycle_policy()).unwrap();
        let rule = &v["rules"][0];
        assert_eq!(v["rules"].as_array().unwrap().len(), 1);
        assert_eq!(rule["rulePriority"], 1);
        assert_eq!(rule["selection"]["tagStatus"], "any");
        assert_eq!(rule["selection"]["countType"], "imageCountMoreThan");
        assert_eq!(rule["selection"]["countNumber"], KEEP_IMAGES);
        assert_eq!(rule["action"]["type"], "expire");
        assert_eq!(KEEP_IMAGES, 3);
    }
}

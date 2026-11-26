use crate::{
    config::{Config, Target},
    ecr::Image,
    progress::{Progress, Stage},
    AppSummary, StatusEntry,
};
use anyhow::{bail, Context, Result};
use aws_sdk_ec2::types::{
    Filter, IamInstanceProfileSpecification, InstanceStateName, ResourceType, Tag, TagSpecification,
};
use std::time::Duration;
use tokio::time::sleep;

const TRUST: &str = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}"#;
const USER_DATA: &str = "#!/bin/bash\ndnf install -y docker\nsystemctl enable --now docker\n";

fn res_name(cfg: &Config) -> String {
    format!("skyship-{}", cfg.name)
}

async fn find_instances(ec2: &aws_sdk_ec2::Client, cfg: &Config) -> Result<Vec<(String, InstanceStateName, Option<String>)>> {
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
        iam.create_role().role_name(&name).assume_role_policy_document(TRUST).send().await?;
        for policy in [
            "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore",
            "arn:aws:iam::aws:policy/AmazonEC2ContainerRegistryReadOnly",
        ] {
            iam.attach_role_policy().role_name(&name).policy_arn(policy).send().await?;
        }
    }
    if iam.get_instance_profile().instance_profile_name(&name).send().await.is_err() {
        iam.create_instance_profile().instance_profile_name(&name).send().await?;
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
    Ok(out.instances().first().and_then(|i| i.instance_id()).context("sem instance id")?.to_string())
}

async fn wait_running(ec2: &aws_sdk_ec2::Client, id: &str) -> Result<String> {
    for _ in 0..60 {
        let out = ec2.describe_instances().instance_ids(id).send().await?;
        let inst = out.reservations().first().and_then(|r| r.instances().first());
        if let Some(i) = inst {
            if i.state().and_then(|s| s.name()) == Some(&InstanceStateName::Running) {
                if let Some(ip) = i.public_ip_address() {
                    return Ok(ip.to_string());
                }
            }
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

/// Devolve a URL pública do app.
pub async fn deploy(sdk: &aws_config::SdkConfig, cfg: &Config, image: &Image, p: &Progress) -> Result<String> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    let ssm = aws_sdk_ssm::Client::new(sdk);

    p.start(Stage::Service);
    let id = ensure_instance(sdk, cfg, p).await?;
    let ip = wait_running(&ec2, &id).await?;
    wait_ssm(&ssm, &id, p).await?;
    p.done(Stage::Service);

    let script = vec![
        "until command -v docker >/dev/null 2>&1 && systemctl is-active --quiet docker; do sleep 3; done".to_string(),
        format!(
            "aws ecr get-login-password --region {} | docker login --username AWS --password-stdin {}",
            cfg.region, image.registry
        ),
        format!("docker pull {}", image.uri),
        "docker rm -f app || true".to_string(),
        format!("docker run -d --name app --restart unless-stopped -p 80:{} {}", cfg.port, image.uri),
        "docker image prune -af --filter until=24h || true".to_string(),
    ];

    p.start(Stage::Publish);
    p.log(format!("executando deploy na instância {id}"));
    let cmd_id = ssm
        .send_command()
        .instance_ids(&id)
        .document_name("AWS-RunShellScript")
        .parameters("commands", script)
        .send()
        .await?
        .command()
        .and_then(|c| c.command_id())
        .context("sem command id")?
        .to_string();

    for _ in 0..120 {
        sleep(Duration::from_secs(3)).await;
        let Ok(inv) = ssm.get_command_invocation().command_id(&cmd_id).instance_id(&id).send().await else {
            continue; // a invocação pode demorar a existir
        };
        match inv.status().map(|s| s.as_str()) {
            Some("Success") => {
                p.done(Stage::Publish);
                p.start(Stage::WaitLive);
                p.done(Stage::WaitLive);
                return Ok(format!("http://{ip}"));
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
    bail!("deploy não terminou a tempo")
}

pub async fn status(sdk: &aws_config::SdkConfig, cfg: &Config) -> Result<Vec<StatusEntry>> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    Ok(find_instances(&ec2, cfg)
        .await?
        .into_iter()
        .map(|(id, state, ip)| StatusEntry { id, state: state.as_str().to_string(), url: ip.map(|i| format!("http://{i}")) })
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
    let mut apps = vec![];
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
                state: i.state().and_then(|s| s.name()).map(|n| n.as_str()).unwrap_or("?").to_string(),
                url: i.public_ip_address().map(|ip| format!("http://{ip}")),
                region: region.to_string(),
                size: i.instance_type().map(|t| t.as_str().to_string()),
                managed: true, // só entram na lista as instâncias com a tag do skyship
            });
        }
    }
    Ok(apps)
}

pub async fn destroy(sdk: &aws_config::SdkConfig, cfg: &Config, p: &Progress) -> Result<()> {
    let ec2 = aws_sdk_ec2::Client::new(sdk);
    let ids: Vec<String> = find_instances(&ec2, cfg).await?.into_iter().map(|i| i.0).collect();
    if ids.is_empty() {
        p.log("nenhuma instância para remover");
        return Ok(());
    }
    p.log(format!("terminando {}", ids.join(", ")));
    ec2.terminate_instances().set_instance_ids(Some(ids)).send().await?;
    p.log(format!("role IAM e security group `{}` foram mantidos; remova à mão se quiser", res_name(cfg)));
    Ok(())
}

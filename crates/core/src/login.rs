//! Credenciais AWS: política IAM necessária, validação das chaves e gravação do perfil.
//! Sem prompts: quem pergunta ao usuário é o CLI ou a GUI.

use crate::config::Target;
use anyhow::{Context, Result, bail};
use aws_credential_types::Credentials;
use std::io::Write;
use std::path::PathBuf;

const POLICY_LIGHTSAIL: &str = r#"{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": [
      "lightsail:GetContainerServices",
      "lightsail:CreateContainerService",
      "lightsail:CreateContainerServiceRegistryLogin",
      "lightsail:RegisterContainerImage",
      "lightsail:CreateContainerServiceDeployment",
      "lightsail:DeleteContainerService",
      "lightsail:TagResource"
    ],
    "Resource": "*"
  }]
}"#;

// Derivada da leitura do código; ainda NÃO testada contra a AWS. Cada permissão vale só para o que o
// skyship cria: repositórios `skyship/*`, roles e profiles `skyship-*` e instâncias com a tag
// `skyship:name`. O que a AWS não deixa limitar por recurso (RunInstances, grupos de segurança,
// leituras) fica em "*".
const POLICY_EC2: &str = r#"{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "EcrLogin",
      "Effect": "Allow",
      "Action": "ecr:GetAuthorizationToken",
      "Resource": "*"
    },
    {
      "Sid": "EcrSkyshipRepositories",
      "Effect": "Allow",
      "Action": [
        "ecr:DescribeRepositories", "ecr:CreateRepository", "ecr:PutLifecyclePolicy", "ecr:BatchCheckLayerAvailability",
        "ecr:InitiateLayerUpload", "ecr:UploadLayerPart", "ecr:CompleteLayerUpload", "ecr:PutImage"
      ],
      "Resource": "arn:aws:ecr:*:*:repository/skyship/*"
    },
    {
      "Sid": "Ec2ReadAndCreate",
      "Effect": "Allow",
      "Action": [
        "ec2:DescribeInstances", "ec2:DescribeInstanceStatus", "ec2:DescribeSecurityGroups", "ec2:RunInstances",
        "ec2:CreateSecurityGroup", "ec2:AuthorizeSecurityGroupIngress"
      ],
      "Resource": "*"
    },
    {
      "Sid": "Ec2TagOnLaunch",
      "Effect": "Allow",
      "Action": "ec2:CreateTags",
      "Resource": "arn:aws:ec2:*:*:instance/*",
      "Condition": { "StringEquals": { "ec2:CreateAction": "RunInstances" } }
    },
    {
      "Sid": "Ec2OnlySkyshipInstances",
      "Effect": "Allow",
      "Action": ["ec2:StartInstances", "ec2:TerminateInstances"],
      "Resource": "arn:aws:ec2:*:*:instance/*",
      "Condition": { "Null": { "ec2:ResourceTag/skyship:name": "false" } }
    },
    {
      "Sid": "IamSkyshipRoleAndProfile",
      "Effect": "Allow",
      "Action": [
        "iam:GetRole", "iam:CreateRole", "iam:GetInstanceProfile",
        "iam:CreateInstanceProfile", "iam:AddRoleToInstanceProfile"
      ],
      "Resource": ["arn:aws:iam::*:role/skyship-*", "arn:aws:iam::*:instance-profile/skyship-*"]
    },
    {
      "Sid": "IamAttachOnlyTheTwoManagedPolicies",
      "Effect": "Allow",
      "Action": "iam:AttachRolePolicy",
      "Resource": "arn:aws:iam::*:role/skyship-*",
      "Condition": {
        "ArnEquals": {
          "iam:PolicyARN": [
            "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore",
            "arn:aws:iam::aws:policy/AmazonEC2ContainerRegistryReadOnly"
          ]
        }
      }
    },
    {
      "Sid": "PassTheRoleOnlyToEc2",
      "Effect": "Allow",
      "Action": "iam:PassRole",
      "Resource": "arn:aws:iam::*:role/skyship-*",
      "Condition": { "StringEquals": { "iam:PassedToService": "ec2.amazonaws.com" } }
    },
    {
      "Sid": "SsmAmiLookup",
      "Effect": "Allow",
      "Action": "ssm:GetParameter",
      "Resource": "arn:aws:ssm:*::parameter/aws/service/ami-amazon-linux-latest/*"
    },
    {
      "Sid": "SsmRunCommand",
      "Effect": "Allow",
      "Action": "ssm:SendCommand",
      "Resource": ["arn:aws:ssm:*:*:document/AWS-RunShellScript", "arn:aws:ec2:*:*:instance/*"]
    },
    {
      "Sid": "SsmRead",
      "Effect": "Allow",
      "Action": ["ssm:DescribeInstanceInformation", "ssm:GetCommandInvocation"],
      "Resource": "*"
    }
  ]
}"#;

/// Política IAM (JSON) necessária para o destino.
pub fn policy(target: Target) -> &'static str {
    match target {
        Target::Lightsail => POLICY_LIGHTSAIL,
        Target::Ec2 => POLICY_EC2,
    }
}

/// Pasta pessoal: `HOME` (Linux e macOS) ou `USERPROFILE` (Windows).
fn home_dir() -> Result<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .context("não achei a pasta pessoal (HOME ou USERPROFILE não definidos)")
}

pub fn credentials_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        return Ok(p.into());
    }
    Ok(home_dir()?.join(".aws").join("credentials"))
}

fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("AWS_CONFIG_FILE") {
        return Ok(p.into());
    }
    Ok(home_dir()?.join(".aws").join("config"))
}

/// Nomes de perfis dentro do texto de `~/.aws/credentials` (`[nome]`) ou de `~/.aws/config`
/// (`[default]` e `[profile nome]`). Só os nomes: as chaves nunca são lidas aqui.
pub fn parse_profile_names(text: &str, is_config: bool) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix('[')?.strip_suffix(']'))
        .map(str::trim)
        .filter_map(|s| {
            if !is_config {
                return Some(s);
            }
            match s.strip_prefix("profile ") {
                Some(name) => Some(name.trim()),
                // `[sso-session x]` e outras seções não são perfis; só `[default]` vale sem prefixo
                None => (s == "default").then_some(s),
            }
        })
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Perfis AWS deste computador: `~/.aws/credentials` e `~/.aws/config`, sem repetição e em ordem.
pub fn list_profiles() -> Vec<String> {
    let mut out = vec![];
    if let Ok(text) = credentials_path().and_then(|p| Ok(std::fs::read_to_string(p)?)) {
        out.extend(parse_profile_names(&text, false));
    }
    if let Ok(text) = config_path().and_then(|p| Ok(std::fs::read_to_string(p)?)) {
        out.extend(parse_profile_names(&text, true));
    }
    out.sort();
    out.dedup();
    out
}

/// Quem são essas chaves na AWS.
#[derive(Debug, Clone)]
pub struct Identity {
    pub account: String,
    pub arn: String,
}

/// Confere o formato e valida as chaves na AWS (`sts:GetCallerIdentity`) sem gravar nada.
pub async fn verify(key_id: &str, secret: &str, region: &str) -> Result<Identity> {
    let key_id = key_id.trim();
    let secret = secret.trim();
    if key_id.len() < 16 || !key_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("Access key ID com formato estranho (esperava algo como AKIA...)");
    }
    if secret.is_empty() {
        bail!("secret vazio");
    }
    let creds = Credentials::new(key_id, secret, None, None, "skyship-login");
    let conf = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .credentials_provider(creds)
        .load()
        .await;
    let id = aws_sdk_sts::Client::new(&conf)
        .get_caller_identity()
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("a AWS recusou essas chaves: {e}"))?;
    Ok(Identity {
        account: id.account().unwrap_or_default().to_string(),
        arn: id.arn().unwrap_or_default().to_string(),
    })
}

/// Grava (ou atualiza) o perfil em `~/.aws/credentials` com permissão 600.
pub fn save_profile(profile: &str, key_id: &str, secret: &str) -> Result<PathBuf> {
    let path = credentials_path()?;
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    write_private(&path, &upsert_profile(&current, profile, key_id.trim(), secret.trim()))?;
    Ok(path)
}

/// Substitui (ou acrescenta) o bloco `[profile]` preservando o resto do arquivo.
pub fn upsert_profile(content: &str, profile: &str, key_id: &str, secret: &str) -> String {
    let header = format!("[{profile}]");
    let block = format!("{header}\naws_access_key_id = {key_id}\naws_secret_access_key = {secret}\n");
    let lines: Vec<&str> = content.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == header) else {
        let mut out = content.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&block);
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    let mut out = String::new();
    for l in &lines[..start] {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&block);
    if end < lines.len() {
        out.push('\n');
        for l in &lines[end..] {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// Grava por um arquivo temporário. No Linux e no macOS o arquivo nasce com permissão 600. No Windows não há
/// esse modo: ele fica na pasta pessoal do usuário, protegida pelas permissões padrão dela.
fn write_private(path: &PathBuf, content: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("skyship-tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(content.as_bytes())?;
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_profile_names, policy, upsert_profile};
    use crate::config::Target;

    #[test]
    fn reads_only_profile_names() {
        let creds = "[default]\naws_access_key_id = AKIA\naws_secret_access_key = SEGREDO\n\n[skyship]\naws_access_key_id = X\n";
        assert_eq!(parse_profile_names(creds, false), vec!["default", "skyship"]);
        let cfg = "[default]\nregion = us-east-1\n\n[profile trabalho]\nsso_session = s\n\n[sso-session s]\nsso_region = us-east-1\n";
        assert_eq!(parse_profile_names(cfg, true), vec!["default", "trabalho"]);
        assert!(parse_profile_names("", true).is_empty());
        // nenhum valor de chave aparece no resultado
        assert!(!parse_profile_names(creds, false).join(",").contains("SEGREDO"));
    }

    #[test]
    fn appends_to_empty_and_keeps_others() {
        let a = upsert_profile("", "skyship", "AKIAAAAAAAAAAAAA", "s1");
        assert!(a.contains("[skyship]\naws_access_key_id = AKIAAAAAAAAAAAAA"));
        let b = upsert_profile("[other]\nk = v\n", "skyship", "AKIAAAAAAAAAAAAA", "s1");
        assert!(b.starts_with("[other]\nk = v\n\n[skyship]"));
    }

    #[test]
    fn replaces_only_its_block() {
        let src = "[a]\nx = 1\n\n[skyship]\naws_access_key_id = OLD\naws_secret_access_key = OLD\n\n[b]\ny = 2\n";
        let out = upsert_profile(src, "skyship", "NEW", "S");
        assert!(out.contains("[a]\nx = 1"));
        assert!(out.contains("[b]\ny = 2"));
        assert!(out.contains("aws_access_key_id = NEW"));
        assert!(!out.contains("OLD"));
    }

    fn statements(target: Target) -> Vec<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_str(policy(target)).expect("a política precisa ser um JSON válido");
        v["Statement"].as_array().expect("Statement").clone()
    }

    fn list(v: &serde_json::Value) -> Vec<String> {
        match v {
            serde_json::Value::String(s) => vec![s.clone()],
            serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn both_policies_are_valid_json() {
        assert!(!statements(Target::Lightsail).is_empty());
        assert!(!statements(Target::Ec2).is_empty());
    }

    #[test]
    fn ec2_policy_never_opens_iam_or_ecr_repositories_to_every_resource() {
        for st in statements(Target::Ec2) {
            let actions = list(&st["Action"]);
            let resources = list(&st["Resource"]);
            let open = resources.iter().any(|r| r == "*");
            for a in &actions {
                if a.starts_with("iam:") {
                    assert!(!open, "{a} não pode valer para todos os recursos");
                }
                // só o login do ECR não aceita limitar por recurso
                if a.starts_with("ecr:") && a != "ecr:GetAuthorizationToken" {
                    assert!(!open, "{a} deve valer só para repositórios do skyship");
                }
            }
        }
    }

    #[test]
    fn pass_role_is_limited_to_skyship_roles_and_the_ec2_service() {
        let pass: Vec<_> = statements(Target::Ec2)
            .into_iter()
            .filter(|s| list(&s["Action"]).iter().any(|a| a == "iam:PassRole"))
            .collect();
        assert_eq!(pass.len(), 1);
        assert_eq!(list(&pass[0]["Resource"]), vec!["arn:aws:iam::*:role/skyship-*"]);
        assert_eq!(
            pass[0]["Condition"]["StringEquals"]["iam:PassedToService"],
            "ec2.amazonaws.com"
        );
    }

    #[test]
    fn ec2_policy_covers_every_action_the_code_calls() {
        // as chamadas que ec2.rs, ecr.rs e lightsail.rs fazem na EC2 (se o código passar a usar outra, entra aqui)
        let allowed: Vec<String> = statements(Target::Ec2)
            .iter()
            .flat_map(|s| list(&s["Action"]))
            .collect();
        for a in [
            "ecr:GetAuthorizationToken",
            "ecr:DescribeRepositories",
            "ecr:PutLifecyclePolicy",
            "ecr:CreateRepository",
            "ecr:PutImage",
            "ec2:DescribeInstances",
            "ec2:DescribeInstanceStatus",
            "ec2:RunInstances",
            "ec2:StartInstances",
            "ec2:TerminateInstances",
            "ec2:CreateTags",
            "ec2:DescribeSecurityGroups",
            "ec2:CreateSecurityGroup",
            "ec2:AuthorizeSecurityGroupIngress",
            "iam:GetRole",
            "iam:CreateRole",
            "iam:AttachRolePolicy",
            "iam:GetInstanceProfile",
            "iam:CreateInstanceProfile",
            "iam:AddRoleToInstanceProfile",
            "iam:PassRole",
            "ssm:GetParameter",
            "ssm:DescribeInstanceInformation",
            "ssm:SendCommand",
            "ssm:GetCommandInvocation",
        ] {
            assert!(allowed.iter().any(|x| x == a), "falta {a} na política da EC2");
        }
    }
}

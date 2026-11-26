//! Verificação opcional de uma imagem local com o Trivy (https://trivy.dev), antes de publicar.
//! O Trivy roda como container do Docker, então não há nada para instalar além do próprio Docker.
//!
//! Só dois tipos de achado: **segredos** dentro da imagem e **vulnerabilidades graves**
//! (HIGH e CRITICAL). É um aviso, nunca um bloqueio, e "nada encontrado" não quer dizer que a
//! imagem é segura. O conteúdo dos segredos NÃO é lido nem devolvido: só a regra e o arquivo.

use crate::config::{Config, valid_image_ref};
use crate::docker;
use crate::progress::Progress;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;

/// Quantos itens de cada tipo voltam para a tela (o total vai nas contagens).
const MAX_ITEMS: usize = 8;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SecretFinding {
    /// Tipo do segredo, como o Trivy o chama (ex.: "AWS Access Key ID").
    pub title: String,
    /// Arquivo dentro da imagem.
    pub file: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VulnFinding {
    pub id: String,
    pub package: String,
    pub installed: String,
    pub fixed: Option<String>,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct ScanReport {
    pub secrets_total: usize,
    pub secrets: Vec<SecretFinding>,
    pub critical: usize,
    pub high: usize,
    /// Quantas das graves já têm versão corrigida.
    pub fixable: usize,
    pub vulns: Vec<VulnFinding>,
}

impl ScanReport {
    pub fn is_clean(&self) -> bool {
        self.secrets_total == 0 && self.critical == 0 && self.high == 0
    }
}

/// Imagem oficial do Trivy usada para a verificação. Fixada numa versão de propósito: o skyship
/// não executa nada que mude sozinho. A primeira verificação baixa esta imagem e o banco de
/// vulnerabilidades (centenas de MB); o banco fica num volume do Docker e é reaproveitado.
pub const TRIVY_IMAGE: &str = "aquasec/trivy:0.58.0";
const CACHE_VOLUME: &str = "skyship-trivy-cache";

/// O Docker responde? É a única coisa de que a verificação precisa.
pub async fn available() -> bool {
    tokio::process::Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Escaneia uma imagem que está no Docker local, rodando o Trivy como container. O container
/// recebe o socket do Docker (para ler a imagem), o que equivale a dar a ele controle do Docker:
/// por isso a imagem do Trivy é a oficial e fica fixada numa versão.
pub async fn scan_image(image: &str) -> Result<ScanReport> {
    if !valid_image_ref(image) {
        bail!("referência de imagem inválida");
    }
    let name = format!("skyship-trivy-{}", std::process::id());
    let fut = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            &name,
            "-v",
            "/var/run/docker.sock:/var/run/docker.sock",
            "-v",
            &format!("{CACHE_VOLUME}:/root/.cache/"),
            TRIVY_IMAGE,
            "image",
            "--quiet",
            "--format",
            "json",
            "--scanners",
            "vuln,secret",
            "--severity",
            "HIGH,CRITICAL",
            "--image-src",
            "docker",
            "--timeout",
            "12m",
            image,
        ])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(Duration::from_secs(15 * 60), fut).await {
        Ok(r) => r.context("não consegui executar o docker")?,
        Err(_) => {
            stop(&name).await;
            bail!("a verificação demorou demais");
        }
    };
    if !out.status.success() {
        stop(&name).await;
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "trivy: {}",
            err.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("falhou")
                .trim()
        );
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Constrói a imagem do projeto com uma tag temporária, escaneia e remove a tag. O build do
/// deploy que vem depois reaproveita o cache das camadas, então o custo extra é pequeno.
pub async fn scan_project(cfg: &Config) -> Result<ScanReport> {
    cfg.validate()?;
    if cfg.image.is_some() {
        bail!("este projeto usa uma imagem pronta; use scan_image");
    }
    let tag = format!("skyship-scan/{}:check", cfg.name);
    docker::build(&tag, &cfg.dockerfile, &cfg.context, &cfg.platform, &Progress::silent()).await?;
    let report = scan_image(&tag).await;
    let _ = tokio::process::Command::new("docker")
        .args(["rmi", &tag])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    report
}

/// Garante que o container do Trivy não fique para trás (cancelamento, tempo esgotado).
async fn stop(name: &str) {
    let _ = tokio::process::Command::new("docker")
        .args(["rm", "-f", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Lê o JSON do Trivy. Ignora de propósito `Match`/`Code` dos segredos, que trazem o trecho do arquivo.
pub fn parse(json: &str) -> Result<ScanReport> {
    let v: Value = serde_json::from_str(json).context("saída do trivy ilegível")?;
    let mut r = ScanReport::default();
    for res in v.get("Results").and_then(Value::as_array).into_iter().flatten() {
        let target = s(res, "Target");
        for sec in res.get("Secrets").and_then(Value::as_array).into_iter().flatten() {
            r.secrets_total += 1;
            if r.secrets.len() < MAX_ITEMS {
                let title = match s(sec, "Title") {
                    t if t.is_empty() => s(sec, "RuleID"),
                    t => t,
                };
                r.secrets.push(SecretFinding {
                    title,
                    file: target.clone(),
                });
            }
        }
        for vu in res
            .get("Vulnerabilities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let sev = s(vu, "Severity");
            match sev.as_str() {
                "CRITICAL" => r.critical += 1,
                "HIGH" => r.high += 1,
                _ => continue,
            }
            let fixed = Some(s(vu, "FixedVersion")).filter(|f| !f.is_empty());
            if fixed.is_some() {
                r.fixable += 1;
            }
            r.vulns.push(VulnFinding {
                id: s(vu, "VulnerabilityID"),
                package: s(vu, "PkgName"),
                installed: s(vu, "InstalledVersion"),
                fixed,
                severity: sev,
            });
        }
    }
    // críticas primeiro, depois as que já têm correção
    r.vulns.sort_by_key(|x| (x.severity != "CRITICAL", x.fixed.is_none()));
    r.vulns.truncate(MAX_ITEMS);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"Results":[
      {"Target":"app (debian 12)","Vulnerabilities":[
        {"VulnerabilityID":"CVE-1","PkgName":"openssl","InstalledVersion":"1.0","FixedVersion":"1.1","Severity":"HIGH"},
        {"VulnerabilityID":"CVE-2","PkgName":"libc","InstalledVersion":"2.0","Severity":"CRITICAL"},
        {"VulnerabilityID":"CVE-3","PkgName":"zlib","InstalledVersion":"3.0","FixedVersion":"3.1","Severity":"MEDIUM"}]},
      {"Target":"app/.env","Secrets":[{"RuleID":"aws-access-key-id","Title":"AWS Access Key ID","Match":"AKIA****SECRET-LINE"}]}]}"#;

    #[test]
    fn counts_only_high_critical_and_secrets() {
        let r = parse(SAMPLE).unwrap();
        assert_eq!((r.critical, r.high, r.fixable, r.secrets_total), (1, 1, 1, 1));
        assert_eq!(r.vulns[0].id, "CVE-2"); // crítica primeiro
        assert_eq!(
            r.secrets[0],
            SecretFinding {
                title: "AWS Access Key ID".into(),
                file: "app/.env".into()
            }
        );
        assert!(!r.is_clean());
    }

    #[test]
    fn secret_content_is_never_returned() {
        let r = parse(SAMPLE).unwrap();
        assert!(!format!("{r:?}").contains("SECRET-LINE"));
    }

    #[test]
    fn empty_results_are_clean_and_junk_is_an_error() {
        assert!(parse(r#"{"Results":null}"#).unwrap().is_clean());
        assert!(parse("não é json").is_err());
    }

    #[test]
    fn caps_the_listed_items() {
        let many: Vec<String> = (0..20)
            .map(|i| {
                format!(r#"{{"VulnerabilityID":"CVE-{i}","PkgName":"p","InstalledVersion":"1","Severity":"HIGH"}}"#)
            })
            .collect();
        let r = parse(&format!(
            r#"{{"Results":[{{"Target":"t","Vulnerabilities":[{}]}}]}}"#,
            many.join(",")
        ))
        .unwrap();
        assert_eq!((r.high, r.vulns.len()), (20, MAX_ITEMS));
    }

    /// Roda o Trivy de verdade (baixa a imagem e o banco). `cargo test -p skyship-core -- --ignored scans_real_image`
    #[test]
    #[ignore]
    fn scans_real_image() {
        let img = std::env::var("SKYSHIP_SCAN_IMAGE").unwrap_or_else(|_| "alpine:3.20".into());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let r = rt.block_on(scan_image(&img)).unwrap();
        println!("{r:#?}");
    }

    /// Constrói e escaneia o projeto de exemplo. `cargo test -p skyship-core scans_real_project -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn scans_real_project() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello");
        let cfg = Config::load_from(&dir).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let r = rt.block_on(scan_project(&cfg)).unwrap();
        println!("critical {} high {} secrets {}", r.critical, r.high, r.secrets_total);
        assert!(
            rt.block_on(docker::image_platform(&format!("skyship-scan/{}:check", cfg.name)))
                .is_err(),
            "a tag temporária deveria ter sido removida"
        );
    }
}

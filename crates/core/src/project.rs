//! Inspeção de uma pasta de projeto (para a interface sugerir nome, porta etc.).

use crate::config::{self, Config};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    /// Caminho canônico da pasta.
    pub dir: String,
    pub has_dockerfile: bool,
    /// Primeira porta declarada em `EXPOSE`, se houver.
    pub port: Option<u16>,
    pub suggested_name: String,
    /// `skyship.toml` existente e válido.
    pub config: Option<Config>,
    /// Motivo, se existe `skyship.toml` mas ele é inválido.
    pub config_error: Option<String>,
}

pub fn inspect(dir: &Path) -> Result<ProjectInfo> {
    let root = dir
        .canonicalize()
        .with_context(|| format!("pasta inacessível: {}", dir.display()))?;
    if !root.is_dir() {
        bail!("não é uma pasta: {}", root.display());
    }

    // O Dockerfile só vale se ficar dentro da pasta (um link para fora não é lido).
    let dockerfile = config::confine(&root, "Dockerfile", "dockerfile")
        .ok()
        .filter(|p| p.is_file());
    let port = dockerfile
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| first_exposed_port(&text));

    let (config, config_error) = if root.join(config::FILE).exists() {
        match Config::load_from(&root) {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        }
    } else {
        (None, None)
    };

    Ok(ProjectInfo {
        dir: root.to_string_lossy().into_owned(),
        has_dockerfile: dockerfile.is_some(),
        port,
        suggested_name: suggest_name(root.file_name().and_then(|s| s.to_str()).unwrap_or("")),
        config,
        config_error,
    })
}

/// Primeira porta numérica de uma linha `EXPOSE` (`80`, `8080/tcp`); variáveis são ignoradas.
pub fn first_exposed_port(dockerfile: &str) -> Option<u16> {
    dockerfile.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("EXPOSE") {
            return None;
        }
        words.find_map(|w| w.split('/').next()?.parse::<u16>().ok().filter(|p| *p > 0))
    })
}

/// Nome de app válido (minúsculas, números e `-`) a partir do nome da pasta.
pub fn suggest_name(folder: &str) -> String {
    let mut out = String::new();
    for c in folder.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "app".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_expose_port() {
        assert_eq!(first_exposed_port("FROM x\nEXPOSE 8080\n"), Some(8080));
        assert_eq!(first_exposed_port("expose 3000/tcp 4000"), Some(3000));
        assert_eq!(first_exposed_port("EXPOSE $PORT\nEXPOSE 80"), Some(80));
        assert_eq!(first_exposed_port("FROM x\nCMD [\"EXPOSE\"]"), None);
        assert_eq!(first_exposed_port("EXPOSE 99999"), None);
    }

    #[test]
    fn suggests_valid_names() {
        assert_eq!(suggest_name("personal-site"), "personal-site");
        assert_eq!(suggest_name("Meu App_Legal!"), "meu-app-legal");
        assert_eq!(suggest_name("___"), "app");
        assert_eq!(suggest_name(""), "app");
        assert!(suggest_name(&"a".repeat(80)).len() <= 40);
        let c = Config::new(
            suggest_name("Meu App_Legal!"),
            "us-east-1".into(),
            80,
            config::Target::Ec2,
        );
        c.validate().unwrap();
    }
}

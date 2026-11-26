//! Configurações e permissões do app, guardadas num arquivo da pasta de configuração do usuário.
//!
//! Ficam do lado do Rust de propósito: assim são os próprios comandos do backend que recusam
//! listar perfis ou o Docker quando a pessoa não permitiu, em vez de confiar só na interface.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Pode listar os nomes dos perfis AWS deste computador. Ligada por padrão.
    #[serde(default = "yes")]
    pub read_profiles: bool,
    /// Pode rodar `docker image ls` e `docker ps` para sugerir o que publicar.
    /// `None` = ainda não perguntado; só `Some(true)` libera.
    #[serde(default)]
    pub docker_list: Option<bool>,
    /// Verifica a imagem escolhida com o Trivy antes de publicar. Desligada por padrão.
    #[serde(default)]
    pub scan_images: bool,
    /// Pode apagar um serviço do Lightsail que não tem a tag do skyship (criado por fora).
    /// Desligado por padrão: a pessoa precisa liberar de propósito.
    #[serde(default)]
    pub delete_unmanaged: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { read_profiles: true, docker_list: None, scan_images: false, delete_unmanaged: false }
    }
}

const FILE: &str = "settings.json";

/// Lê as configurações; arquivo ausente ou ilegível vira o padrão (que é o mais restritivo
/// para o que exige permissão).
pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, s: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("skyship-settings-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults_are_restrictive_for_docker() {
        let s = load(&tmp());
        assert!(s.read_profiles);
        assert_eq!(s.docker_list, None);
        assert!(!s.delete_unmanaged, "apagar o que não é do skyship começa desligado");
        assert!(!s.scan_images);
    }

    #[test]
    fn round_trip() {
        let d = tmp();
        let s = Settings { read_profiles: false, docker_list: Some(true), scan_images: true, delete_unmanaged: true };
        save(&d, &s).unwrap();
        assert_eq!(load(&d), s);
    }

    #[test]
    fn broken_or_partial_files_fall_back_safely() {
        let d = tmp();
        std::fs::write(d.join(FILE), "isto não é json").unwrap();
        assert_eq!(load(&d), Settings::default());
        std::fs::write(d.join(FILE), r#"{"readProfiles": false}"#).unwrap();
        let s = load(&d);
        assert!(!s.read_profiles);
        assert_eq!(s.docker_list, None, "campo ausente nunca vira permissão");
    }

    #[test]
    fn serializes_in_camel_case_for_the_frontend() {
        let j = serde_json::to_string(&Settings { read_profiles: true, docker_list: Some(false), scan_images: false, delete_unmanaged: false }).unwrap();
        assert!(j.contains("readProfiles") && j.contains("dockerList"));
    }
}

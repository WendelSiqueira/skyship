use crate::progress::Progress;
use anyhow::{Context, Result, bail};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub async fn available() -> bool {
    Command::new("docker")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub async fn build(image: &str, dockerfile: &str, context: &str, platform: &str, p: &Progress) -> Result<()> {
    run(
        Command::new("docker").args([
            "build",
            "--progress=plain",
            "--platform",
            platform,
            "-f",
            dockerfile,
            "-t",
            image,
            context,
        ]),
        p,
    )
    .await
}

/// `os/arquitetura` de uma imagem local (ex.: `linux/amd64`).
pub async fn image_platform(image: &str) -> Result<String> {
    let out = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Os}}/{{.Architecture}}", image])
        .output()
        .await
        .context("falha ao executar docker")?;
    if !out.status.success() {
        bail!("a imagem {image} não foi encontrada neste computador");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `linux/amd64` casa com `linux/amd64` (variantes como `/v8` são ignoradas).
pub fn platform_matches(wanted: &str, actual: &str) -> bool {
    let mut w = wanted.split('/');
    let mut a = actual.split('/');
    matches!((w.next(), w.next(), a.next(), a.next()), (Some(w0), Some(w1), Some(a0), Some(a1)) if w0 == a0 && w1 == a1)
}

/// Para publicar uma imagem que já existe: ela precisa rodar no destino (hoje, `linux/amd64`).
pub async fn ensure_platform(image: &str, wanted: &str, p: &Progress) -> Result<()> {
    let actual = image_platform(image).await?;
    if !platform_matches(wanted, &actual) {
        bail!(
            "a imagem {image} é {actual}, mas o destino roda {wanted}. Reconstrua a imagem para {wanted} \
             (por exemplo: docker build --platform {wanted} ...) ou publique pela pasta do projeto."
        );
    }
    p.log(format!("usando a imagem {image} ({actual}), sem build"));
    Ok(())
}

pub async fn tag(src: &str, dst: &str, p: &Progress) -> Result<()> {
    run(Command::new("docker").args(["tag", src, dst]), p).await
}

pub async fn push(image: &str, p: &Progress) -> Result<()> {
    run(Command::new("docker").args(["push", image]), p).await
}

/// Digest (sha256:...) que o registry atribuiu à imagem depois do push.
pub async fn repo_digest(image: &str, registry: &str) -> Result<String> {
    let out = Command::new("docker")
        .args([
            "image",
            "inspect",
            image,
            "--format",
            "{{range .RepoDigests}}{{println .}}{{end}}",
        ])
        .output()
        .await
        .context("falha ao executar docker")?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|l| l.starts_with(registry))
        .and_then(|l| l.split_once('@'))
        .map(|(_, d)| d.to_string())
        .context("não achei o digest da imagem após o push")
}

pub async fn login(registry: &str, user: &str, password: &str, p: &Progress) -> Result<()> {
    let mut child = Command::new("docker")
        .args(["login", "--username", user, "--password-stdin", registry])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("falha ao executar docker")?;
    let mut stdin = child.stdin.take().context("sem stdin")?;
    stdin.write_all(password.as_bytes()).await?;
    drop(stdin);
    finish(child, p).await.context("docker login falhou")
}

async fn run(cmd: &mut Command, p: &Progress) -> Result<()> {
    // kill_on_drop: se o deploy for cancelado (a tarefa abortada), o docker morre junto
    let child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("falha ao executar docker")?;
    finish(child, p).await
}

/// Encaminha stdout/stderr linha a linha como eventos de log e espera o processo terminar.
async fn finish(mut child: tokio::process::Child, p: &Progress) -> Result<()> {
    let out = child.stdout.take().map(|s| tokio::spawn(forward(s, p.clone())));
    let err = child.stderr.take().map(|s| tokio::spawn(forward(s, p.clone())));
    let status = child.wait().await?;
    if let Some(t) = out {
        let _ = t.await;
    }
    if let Some(t) = err {
        let _ = t.await;
    }
    if !status.success() {
        bail!("docker terminou com {status}");
    }
    Ok(())
}

async fn forward<R: AsyncRead + Unpin>(r: R, p: Progress) {
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        p.log(line);
    }
}

#[cfg(test)]
mod tests {
    use super::platform_matches;

    #[test]
    fn platform_comparison() {
        assert!(platform_matches("linux/amd64", "linux/amd64"));
        assert!(!platform_matches("linux/amd64", "linux/arm64"));
        assert!(!platform_matches("linux/amd64", "windows/amd64"));
        assert!(!platform_matches("linux", "linux/amd64"));
        assert!(!platform_matches("linux/amd64", ""));
    }
}

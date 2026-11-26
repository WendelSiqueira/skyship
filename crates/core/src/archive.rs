//! Abre um projeto que veio dentro de um arquivo compactado (`.zip`, `.tar`, `.tar.gz`/`.tgz`; e
//! `.rar` quando o sistema tem o `bsdtar`).
//!
//! O arquivo é tratado como entrada não confiável: caminhos que escapam da pasta de destino,
//! links simbólicos e arquivos especiais são recusados ou descartados, e há limites de tamanho e de
//! quantidade de arquivos contra "bombas de descompressão".

use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Total de bytes que podem ser gravados.
    pub max_bytes: u64,
    pub max_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_files: 20_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    TarGz,
    Rar,
}

/// Formato pelo nome do arquivo.
pub fn kind(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".zip") {
        Some(Kind::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(Kind::TarGz)
    } else if name.ends_with(".tar") {
        Some(Kind::Tar)
    } else if name.ends_with(".rar") {
        Some(Kind::Rar)
    } else {
        None
    }
}

/// Cria uma pasta nova e privada em `/tmp` (ou equivalente) para receber a extração.
pub fn new_dest_dir() -> Result<PathBuf> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "skyship-archive-{}-{nanos}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let mut b = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&dir)
        .with_context(|| format!("não consegui criar {}", dir.display()))?;
    Ok(dir)
}

/// Remove extrações antigas (`skyship-archive-*`) deixadas na pasta temporária.
pub fn cleanup_old() {
    let Ok(rd) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with("skyship-archive-") {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

/// Extrai `archive` em `dest` (pasta vazia que já existe) e devolve a pasta do projeto: a única
/// pasta de topo, se o arquivo tem uma só, ou o próprio `dest`.
pub fn extract(archive: &Path, dest: &Path, limits: Limits) -> Result<PathBuf> {
    let k = kind(archive).context("formato não suportado (use .zip, .tar, .tar.gz, .tgz ou .rar)")?;
    match k {
        Kind::Zip => extract_zip(archive, dest, limits)?,
        Kind::Tar => extract_tar(File::open(archive)?, dest, limits)?,
        Kind::TarGz => extract_tar(flate2::read::GzDecoder::new(File::open(archive)?), dest, limits)?,
        Kind::Rar => extract_rar(archive, dest)?,
    }
    // Segunda linha de defesa, para qualquer formato: nada de links simbólicos e dentro dos limites.
    sanitize(dest, limits)?;
    Ok(project_root(dest))
}

fn project_root(dest: &Path) -> PathBuf {
    let entries: Vec<_> = fs::read_dir(dest).map(|r| r.flatten().collect()).unwrap_or_default();
    if entries.len() == 1 && entries[0].path().is_dir() {
        entries[0].path()
    } else {
        dest.to_path_buf()
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    // só rwx de dono/grupo/outros: sem setuid, setgid nem sticky
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777));
}
#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) {}

fn extract_zip(archive: &Path, dest: &Path, limits: Limits) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).context("zip inválido")?;
    if zip.len() > limits.max_files {
        bail!(
            "o arquivo tem arquivos demais ({}; o limite é {})",
            zip.len(),
            limits.max_files
        );
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let Some(rel) = f.enclosed_name() else {
            bail!(
                "caminho inseguro dentro do arquivo: {}",
                f.name().map(|n| n.to_string()).unwrap_or_default()
            );
        };
        if f.is_symlink() {
            continue; // links simbólicos não são extraídos
        }
        let out = dest.join(&rel);
        if f.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mode = f.unix_mode();
        let mut w = File::create(&out)?;
        // conta o que de fato foi gravado, sem confiar no tamanho declarado
        let room = limits.max_bytes - total;
        let n = io::copy(&mut (&mut f).take(room + 1), &mut w)?;
        total += n;
        if total > limits.max_bytes {
            bail!(
                "o conteúdo descompactado passa do limite de {} MB",
                limits.max_bytes >> 20
            );
        }
        if let Some(m) = mode {
            set_mode(&out, m);
        }
    }
    Ok(())
}

fn extract_tar<R: Read>(reader: R, dest: &Path, limits: Limits) -> Result<()> {
    use tar::EntryType;
    let mut ar = tar::Archive::new(reader);
    ar.set_preserve_permissions(false);
    ar.set_unpack_xattrs(false);
    let (mut total, mut files) = (0u64, 0usize);
    for entry in ar.entries().context("tar inválido")? {
        let mut entry = entry?;
        let ty = entry.header().entry_type();
        if !matches!(ty, EntryType::Regular | EntryType::Directory | EntryType::Continuous) {
            continue; // links, dispositivos, fifos e extensões: ignorados
        }
        files += 1;
        if files > limits.max_files {
            bail!("o arquivo tem arquivos demais (o limite é {})", limits.max_files);
        }
        total = total.saturating_add(entry.header().size().unwrap_or(0));
        if total > limits.max_bytes {
            bail!(
                "o conteúdo descompactado passa do limite de {} MB",
                limits.max_bytes >> 20
            );
        }
        let mode = entry.header().mode().ok();
        let rel = entry.path()?.into_owned();
        // `unpack_in` devolve false quando o caminho sairia de `dest` (`..`, absoluto)
        if !entry.unpack_in(dest)? {
            bail!("caminho inseguro dentro do arquivo: {}", rel.display());
        }
        if ty != EntryType::Directory
            && let Some(m) = mode
        {
            set_mode(&dest.join(&rel), m);
        }
    }
    Ok(())
}

/// RAR é um formato proprietário: usa o `bsdtar` (libarchive) se o sistema tiver. A extração roda
/// com tempo limite e depois passa pelo mesmo `sanitize`.
fn extract_rar(archive: &Path, dest: &Path) -> Result<()> {
    let child = std::process::Command::new("bsdtar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            anyhow::anyhow!("para abrir .rar é preciso o `bsdtar` (pacote libarchive) instalado; ou use .zip / .tar.gz")
        })?;
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let out = rx
        .recv_timeout(Duration::from_secs(120))
        .map_err(|_| anyhow::anyhow!("o bsdtar demorou demais para abrir o arquivo"))??;
    let _ = handle.join();
    if !out.status.success() {
        bail!(
            "não consegui abrir o .rar: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Percorre a pasta extraída: apaga links simbólicos e confere os limites.
fn sanitize(dir: &Path, limits: Limits) -> Result<()> {
    fn walk(dir: &Path, limits: Limits, bytes: &mut u64, files: &mut usize) -> Result<()> {
        for e in fs::read_dir(dir)? {
            let e = e?;
            let meta = fs::symlink_metadata(e.path())?;
            if meta.file_type().is_symlink() {
                fs::remove_file(e.path())?;
            } else if meta.is_dir() {
                walk(&e.path(), limits, bytes, files)?;
            } else {
                *files += 1;
                *bytes += meta.len();
                if *files > limits.max_files || *bytes > limits.max_bytes {
                    bail!(
                        "o conteúdo extraído passa dos limites (arquivos: {files}, MB: {})",
                        *bytes >> 20
                    );
                }
            }
        }
        Ok(())
    }
    walk(dir, limits, &mut 0, &mut 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp() -> PathBuf {
        new_dest_dir().unwrap()
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn make_targz(path: &Path, entries: &[(&str, &[u8])]) {
        let gz = flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::default());
        let mut b = tar::Builder::new(gz);
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn detects_kind_by_name() {
        assert_eq!(kind(Path::new("a.ZIP")), Some(Kind::Zip));
        assert_eq!(kind(Path::new("a.tar.gz")), Some(Kind::TarGz));
        assert_eq!(kind(Path::new("a.tgz")), Some(Kind::TarGz));
        assert_eq!(kind(Path::new("a.tar")), Some(Kind::Tar));
        assert_eq!(kind(Path::new("a.rar")), Some(Kind::Rar));
        assert_eq!(kind(Path::new("a.exe")), None);
    }

    #[test]
    fn zip_with_single_root_folder_returns_that_folder() {
        let d = tmp();
        let z = d.join("p.zip");
        make_zip(
            &z,
            &[
                ("meuapp/Dockerfile", b"FROM scratch\n"),
                ("meuapp/index.html", b"<h1>oi</h1>"),
            ],
        );
        let out = tmp();
        let root = extract(&z, &out, Limits::default()).unwrap();
        assert!(root.ends_with("meuapp"));
        assert!(root.join("Dockerfile").is_file());
    }

    #[test]
    fn zip_with_files_at_top_level_returns_dest() {
        let d = tmp();
        let z = d.join("p.zip");
        make_zip(&z, &[("Dockerfile", b"FROM scratch\n"), ("a.txt", b"x")]);
        let out = tmp();
        let root = extract(&z, &out, Limits::default()).unwrap();
        assert_eq!(root, out);
    }

    #[test]
    fn targz_extracts() {
        let d = tmp();
        let t = d.join("p.tar.gz");
        make_targz(&t, &[("app/Dockerfile", b"FROM scratch\n")]);
        let out = tmp();
        let root = extract(&t, &out, Limits::default()).unwrap();
        assert!(root.join("Dockerfile").is_file());
    }

    #[test]
    fn zip_slip_is_rejected() {
        let d = tmp();
        let z = d.join("evil.zip");
        make_zip(&z, &[("../fora.txt", b"x")]);
        let out = tmp();
        assert!(extract(&z, &out, Limits::default()).is_err());
        assert!(!out.parent().unwrap().join("fora.txt").exists());
    }

    #[test]
    fn tar_path_traversal_is_rejected() {
        let d = tmp();
        let t = d.join("evil.tar");
        // monta o cabeçalho na mão: o Builder não deixa criar `..`
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_mode(0o644);
        let name = b"../fora.txt";
        h.as_old_mut().name[..name.len()].copy_from_slice(name);
        h.set_cksum();
        let mut f = File::create(&t).unwrap();
        f.write_all(h.as_bytes()).unwrap();
        f.write_all(&[b'x'; 512]).unwrap();
        f.write_all(&[0u8; 1024]).unwrap();
        drop(f);
        let out = tmp();
        assert!(extract(&t, &out, Limits::default()).is_err());
        assert!(!out.parent().unwrap().join("fora.txt").exists());
    }

    #[test]
    fn size_and_count_limits_apply() {
        let d = tmp();
        let z = d.join("big.zip");
        make_zip(&z, &[("a", &[0u8; 5000]), ("b", &[0u8; 5000])]);
        let out = tmp();
        assert!(
            extract(
                &z,
                &out,
                Limits {
                    max_bytes: 6000,
                    max_files: 10
                }
            )
            .is_err()
        );
        let out2 = tmp();
        assert!(
            extract(
                &z,
                &out2,
                Limits {
                    max_bytes: 1 << 20,
                    max_files: 1
                }
            )
            .is_err()
        );
    }

    #[test]
    fn symlinks_inside_tar_are_dropped() {
        let d = tmp();
        let t = d.join("l.tar");
        let mut b = tar::Builder::new(File::create(&t).unwrap());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        b.append_link(&mut h, "link", "/etc/passwd").unwrap();
        let mut h2 = tar::Header::new_gnu();
        h2.set_size(2);
        h2.set_mode(0o644);
        h2.set_cksum();
        b.append_data(&mut h2, "Dockerfile", &b"ok"[..]).unwrap();
        b.finish().unwrap();
        let out = tmp();
        extract(&t, &out, Limits::default()).unwrap();
        assert!(out.join("Dockerfile").is_file());
        assert!(!out.join("link").exists());
    }

    #[test]
    fn unknown_extension_is_rejected() {
        let d = tmp();
        let f = d.join("a.exe");
        File::create(&f).unwrap();
        assert!(extract(&f, &tmp(), Limits::default()).is_err());
    }
}

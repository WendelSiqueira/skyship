// Sem janela de console no Windows em release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Interface gráfica do skyship. Só traduz cliques em chamadas ao `skyship-core` e repassa os
//! eventos de progresso para a janela; nenhuma regra de deploy fica aqui.

mod settings;

use serde::{Deserialize, Serialize};
use settings::Settings;
use skyship_core::{
    Aws, ListResult, StatusEntry,
    archive::{self, Limits},
    config::{Config, Target},
    docker, local,
    local::LocalDocker,
    login,
    progress::{Event, Progress},
    project::{self, ProjectInfo},
    regions::{REGIONS, RegionInfo},
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Deploy em andamento (no máximo um por vez); guardado para poder cancelar.
struct DeployState(Mutex<Option<tauri::async_runtime::JoinHandle<()>>>);

#[derive(Serialize)]
struct Account {
    account: String,
    arn: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigInput {
    name: String,
    region: String,
    port: u16,
    target: Target,
    instance_type: String,
    power: String,
    /// Imagem Docker local a publicar (modo "imagens do Docker"); `None` = build pela pasta.
    #[serde(default)]
    image: Option<String>,
}

fn config_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("skyship"))
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    settings::load(&config_dir(&app))
}

/// Liga ou desliga permissões. Campos ausentes não mudam.
#[tauri::command]
fn update_settings(
    app: AppHandle,
    read_profiles: Option<bool>,
    docker_list: Option<bool>,
    scan_images: Option<bool>,
    delete_unmanaged: Option<bool>,
) -> CmdResult<Settings> {
    let dir = config_dir(&app);
    let mut s = settings::load(&dir);
    if let Some(v) = read_profiles {
        s.read_profiles = v;
    }
    if let Some(v) = docker_list {
        s.docker_list = Some(v);
    }
    if let Some(v) = scan_images {
        s.scan_images = v;
    }
    if let Some(v) = delete_unmanaged {
        s.delete_unmanaged = v;
    }
    settings::save(&dir, &s).map_err(|e| e.to_string())?;
    Ok(s)
}

/// Imagens com tag e containers em execução deste computador. Só roda com a permissão ligada.
#[tauri::command]
async fn list_local_docker(app: AppHandle) -> CmdResult<LocalDocker> {
    if settings::load(&config_dir(&app)).docker_list != Some(true) {
        return Err("permission".into());
    }
    local::list().await.map_err(err)
}

/// O Docker responde? O Trivy roda como container, então é só isso que a verificação precisa.
#[tauri::command]
async fn trivy_available() -> bool {
    skyship_core::scan::available().await
}

/// Verifica uma imagem local com o Trivy (segredos e vulnerabilidades graves). Só roda se a pessoa
/// ligou a verificação nas configurações.
#[tauri::command]
async fn scan_image(app: AppHandle, image: String) -> CmdResult<skyship_core::scan::ScanReport> {
    if !settings::load(&config_dir(&app)).scan_images {
        return Err("permission".into());
    }
    skyship_core::scan::scan_image(&image).await.map_err(err)
}

/// Constrói o projeto (tag temporária) e o verifica com o Trivy. Só roda com a verificação ligada.
#[tauri::command]
async fn scan_project(app: AppHandle, dir: String) -> CmdResult<skyship_core::scan::ScanReport> {
    if !settings::load(&config_dir(&app)).scan_images {
        return Err("permission".into());
    }
    let cfg = Config::load_from(Path::new(&dir)).map_err(err)?;
    skyship_core::scan::scan_project(&cfg).await.map_err(err)
}

/// Abre um arquivo compactado numa pasta temporária e devolve a análise do projeto dentro dele.
#[tauri::command]
async fn extract_archive(path: String) -> CmdResult<ProjectInfo> {
    tauri::async_runtime::spawn_blocking(move || {
        let dest = archive::new_dest_dir()?;
        match archive::extract(Path::new(&path), &dest, Limits::default()) {
            Ok(root) => project::inspect(&root),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err)
}

#[tauri::command]
fn regions() -> &'static [RegionInfo] {
    REGIONS
}

#[tauri::command]
fn policy(target: Target) -> &'static str {
    login::policy(target)
}

#[tauri::command]
fn profiles(app: AppHandle) -> Vec<String> {
    // sem a permissão, nem os nomes dos perfis são lidos
    if !settings::load(&config_dir(&app)).read_profiles {
        return vec![];
    }
    login::list_profiles()
}

#[tauri::command]
async fn docker_ok() -> bool {
    docker::available().await
}

/// Valida as chaves na AWS e só então grava o perfil em `~/.aws/credentials`.
#[tauri::command]
async fn save_credentials(profile: String, key_id: String, secret: String, region: String) -> CmdResult<Account> {
    if profile.is_empty()
        || !profile
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("nome de perfil inválido (use letras, números, - e _)".into());
    }
    let id = login::verify(&key_id, &secret, &region).await.map_err(err)?;
    login::save_profile(&profile, &key_id, &secret).map_err(err)?;
    Ok(Account {
        account: id.account,
        arn: id.arn,
    })
}

/// Confere se um perfil existente tem credenciais válidas; devolve o número da conta.
#[tauri::command]
async fn check_profile(profile: Option<String>, region: String) -> CmdResult<String> {
    if !skyship_core::regions::is_known(&region) {
        return Err("região desconhecida".into());
    }
    Aws::connect(&region, profile.as_deref())
        .await
        .account()
        .await
        .map_err(err)
}

#[tauri::command]
fn inspect_project(dir: String) -> CmdResult<ProjectInfo> {
    project::inspect(Path::new(&dir)).map_err(err)
}

/// Grava o `skyship.toml` e devolve a pasta onde ele ficou. Preserva `context`/`dockerfile` de um
/// arquivo já existente. No modo "imagem" não há pasta do projeto: a config vai para uma pasta
/// própria do app (`<config>/apps/<nome>`).
#[tauri::command]
fn save_config(app: AppHandle, dir: Option<String>, input: ConfigInput) -> CmdResult<String> {
    // o nome entra num caminho no modo imagem: valida antes de usar
    Config::new(input.name.clone(), input.region.clone(), input.port, input.target)
        .validate()
        .map_err(err)?;
    let dir: PathBuf = if input.image.is_some() {
        let d = config_dir(&app).join("apps").join(&input.name);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        d
    } else {
        PathBuf::from(dir.ok_or("escolha a pasta do projeto")?)
    };
    let mut cfg = Config::load_unresolved(&dir)
        .unwrap_or_else(|_| Config::new(input.name.clone(), input.region.clone(), input.port, input.target));
    cfg.name = input.name;
    cfg.region = input.region;
    cfg.port = input.port;
    cfg.target = input.target;
    cfg.image = input.image;
    cfg.ec2.instance_type = input.instance_type;
    cfg.lightsail.power = input.power;
    cfg.write_to(&dir).map_err(err)?;
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
async fn start_deploy(
    app: AppHandle,
    state: State<'_, DeployState>,
    dir: String,
    profile: Option<String>,
) -> CmdResult<()> {
    let cfg = Config::load_from(Path::new(&dir)).map_err(err)?;
    let mut slot = state.0.lock().unwrap();
    if slot.is_some() {
        return Err("já existe um deploy em andamento".into());
    }
    let handle = tauri::async_runtime::spawn(async move {
        let aws = Aws::connect(&cfg.region, profile.as_deref()).await;
        let emitter = app.clone();
        let progress = Progress::new(move |e| {
            let _ = emitter.emit("deploy-event", &e);
        });
        if let Err(e) = skyship_core::deploy(&aws, &cfg, &progress).await {
            progress.emit(Event::Failed {
                message: format!("{e:#}"),
            });
        }
        // Libera o slot; start_deploy segura o lock até guardar o handle, então não há corrida.
        *app.state::<DeployState>().0.lock().unwrap() = None;
    });
    *slot = Some(handle);
    Ok(())
}

#[tauri::command]
fn cancel_deploy(app: AppHandle, state: State<'_, DeployState>) {
    if let Some(handle) = state.0.lock().unwrap().take() {
        // Abortar a tarefa derruba também o docker em andamento (kill_on_drop).
        handle.abort();
        let _ = app.emit("deploy-event", &Event::Cancelled);
    }
}

#[tauri::command]
async fn status(dir: String, profile: Option<String>) -> CmdResult<Vec<StatusEntry>> {
    let cfg = Config::load_from(Path::new(&dir)).map_err(err)?;
    let aws = Aws::connect(&cfg.region, profile.as_deref()).await;
    skyship_core::status(&aws, &cfg).await.map_err(err)
}

/// Apps encontrados na região (Lightsail e EC2). Funciona sem um projeto aberto.
#[tauri::command]
async fn list_apps(profile: Option<String>, region: String) -> CmdResult<ListResult> {
    if !skyship_core::regions::is_known(&region) {
        return Err("região desconhecida".into());
    }
    let aws = Aws::connect(&region, profile.as_deref()).await;
    Ok(skyship_core::list_apps(&aws).await)
}

/// Não pergunta nada: a confirmação é feita na interface antes de chamar.
#[tauri::command]
async fn destroy(dir: String, profile: Option<String>) -> CmdResult<()> {
    let cfg = Config::load_from(Path::new(&dir)).map_err(err)?;
    let aws = Aws::connect(&cfg.region, profile.as_deref()).await;
    skyship_core::destroy(&aws, &cfg, &Progress::silent())
        .await
        .map_err(err)
}

/// Remove um app da lista da conta (sem pasta de projeto). A confirmação é feita na interface.
#[tauri::command]
async fn destroy_app(
    app: AppHandle,
    profile: Option<String>,
    region: String,
    target: String,
    name: String,
) -> CmdResult<()> {
    let allow_unmanaged = settings::load(&config_dir(&app)).delete_unmanaged;
    let target: skyship_core::config::Target = target.parse()?;
    let aws = Aws::connect(&region, profile.as_deref()).await;
    skyship_core::destroy_app(&aws, &name, &region, target, allow_unmanaged, &Progress::silent())
        .await
        .map_err(err)
}

fn main() {
    // WebKitGTK com Wayland e placa NVIDIA derruba o app logo na abertura ("Error 71 (Protocol
    // error) dispatching to Wayland display"). Desligar o renderizador DMABUF evita isso e custa
    // pouco numa interface simples. Respeita quem já definiu a variável.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: roda no início do `main`, antes de qualquer outra thread existir.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }

    // extrações de arquivos compactados de execuções anteriores
    archive::cleanup_old();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(DeployState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            regions,
            policy,
            profiles,
            docker_ok,
            save_credentials,
            check_profile,
            inspect_project,
            save_config,
            start_deploy,
            cancel_deploy,
            status,
            list_apps,
            destroy,
            destroy_app,
            get_settings,
            update_settings,
            list_local_docker,
            trivy_available,
            scan_image,
            scan_project,
            extract_archive,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o skyship");
}

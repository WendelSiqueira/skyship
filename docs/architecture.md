# Architecture

skyship is a Cargo workspace: the logic lives in a library, and the interfaces only call it.

```
crates/
  core/   skyship-core   all the deploy logic, no user interface
  cli/    skyship        command line (clap): prompts and terminal output
  gui/    skyship-gui    desktop app (Tauri 2): the same steps, on screen
```

## The core rule

`skyship-core` never writes to the terminal and never asks the user anything. Everything that is
progress leaves as a `progress::Event`, and whoever calls it (the CLI, or any other interface)
decides how to show it:

- `StageStarted { stage }` / `StageDone { stage }` with `Stage` one of `Build`, `Service`,
  `Upload`, `Publish`, `WaitLive` (the five steps of a deploy)
- `Log { line }`: technical output (the `docker` output, details of what is being created)
- `Done { url }`, `Failed { message }`, `Cancelled`

Public API (`crates/core/src/lib.rs`): `Aws::connect`, `deploy`, `status`, `destroy`, plus
`config`, `login`, `project`, `regions` and `progress`.

## Files

| File | Role |
|---|---|
| `crates/core/src/lib.rs` | Public API |
| `crates/core/src/progress.rs` | `Stage`, `Event`, `Progress` |
| `crates/core/src/config.rs` | `skyship.toml`: types, defaults, validation, path containment |
| `crates/core/src/regions.rs` | Closed list of accepted regions |
| `crates/core/src/project.rs` | Inspects a folder (Dockerfile, `EXPOSE` port, suggested name) |
| `crates/core/src/docker.rs` | Calls the `docker` binary and forwards its output as log events |
| `crates/core/src/login.rs` | IAM policies, key verification, profile saving (no prompts) |
| `crates/core/src/ecr.rs` | ECR repository + build/login/push (EC2 only) |
| `crates/core/src/ec2.rs` | Role, security group, instance, deploy over SSM |
| `crates/core/src/lightsail.rs` | Service, Lightsail registry, deployment |
| `crates/cli/src/main.rs` | The `skyship` commands |
| `crates/gui/src/main.rs` | Tauri commands that call the core and forward its events to the window |
| `crates/gui/ui/` | The screens: `index.html`, `style.css`, `app.js`, `i18n.js` (no Node, no bundler) |

Decisions: the **AWS SDK for Rust** is used (one binary, no AWS CLI needed), and **`docker` is
called as a process** instead of talking to the Docker API.

## Configuration (`skyship.toml`)

```toml
name = "myapp"             # lowercase letters, digits and '-' (up to 40 characters)
region = "us-east-2"       # must be in the list shown by `skyship regions`
port = 80                  # the port the app listens on inside the container
dockerfile = "Dockerfile"  # relative to the project folder
context = "."              # relative to the project folder
platform = "linux/amd64"   # forces amd64 even when building on an ARM machine
target = "lightsail"       # or "ec2"
# image = "myapp:1.0"      # optional: publish an image that already exists locally (no build);
                           # dockerfile and context are then ignored

[ec2]
instance_type = "t3.micro"

[lightsail]
power = "nano"
scale = 1
```

## How each destination works

### Lightsail (`lightsail.rs`)

No ECR: it uses the Lightsail registry of the service itself.

1. `docker build` locally.
2. Create the container service if needed and wait until it is ready (the service must exist
   before an image can be registered, which is why `Service` comes before `Upload`).
3. `CreateContainerServiceRegistryLogin` → `docker login` → `docker push`.
4. `RegisterContainerImage` with the digest returns the image name (`:service.app.N`).
5. `CreateContainerServiceDeployment`, then wait until the deployment is `ACTIVE`. Returns the URL.

### EC2 (`ec2.rs` + `ecr.rs`)

Uses ECR as the registry and SSM to run commands (no SSH, no key pair, only port 80 open).

The stages run in the order the screen shows them: Build, Service, Upload, Publish, WaitLive.

1. **Build** (`ecr::build`): ensure the `skyship/<name>` repository and build the image locally
   (tag = timestamp). Nothing is sent yet.
2. **Service** (`ec2::prepare`): ensure the IAM role + instance profile `skyship-<name>` (SSM +
   read access to ECR) and the security group `skyship-<name>` (port 80). Find the instance by its
   `skyship:name` tag; if none exists, launch one (Amazon Linux 2023, the user data installs
   Docker). Wait until it is `running` and the SSM agent is online, and log its status checks.
3. **Upload** (`ecr::push`): log in to ECR and push the image.
4. **Publish** (`ec2::publish`): `SendCommand` (`AWS-RunShellScript`): log in to ECR, `docker pull`,
   recreate the `app` container with `-p 80:<port>` and `--restart unless-stopped`.
5. **WaitLive**: poll `http://<ip>/` until it answers with any HTTP status (up to 5 minutes), logging
   the instance status checks (`ok`, `initializing`, `impaired`) every 15 seconds. The address is
   plain `http://`: the instance has no TLS and only port 80 is open.

`destroy` for EC2 terminates the instance but keeps the IAM role and the security group. Each
deploy pushes a new image under a new tag (a timestamp) and sets a lifecycle policy on the
`skyship/<name>` repository that keeps only the 3 most recent images (`ecr::KEEP_IMAGES`); ECR deletes
the older ones asynchronously, within about a day. Without `ecr:PutLifecyclePolicy` the deploy still
works and the log says the cleanup was not configured.

## Cancelling

Aborting the task that runs `deploy` also kills the running `docker` process (`kill_on_drop`), so a
cancelled build does not keep running in the background.

## The desktop app

`crates/gui` is a Tauri app. The frontend is plain HTML, CSS and JavaScript in `crates/gui/ui`,
loaded from local files only (`withGlobalTauri` exposes the Tauri API; the content security policy
allows scripts only from the app itself).

- `src/main.rs` exposes the core as Tauri commands (`save_credentials`, `check_profile`,
  `inspect_project`, `save_config`, `start_deploy`, `cancel_deploy`, `status`, `destroy`, ...). It
  holds no deploy logic.
- `start_deploy` runs the deploy in a background task and emits every `progress::Event` to the
  window as `deploy-event`. `cancel_deploy` aborts that task.
- Three sources for the app, chosen with a radio on the project screen: a **project folder** (build
  from its `Dockerfile`), an **image from the local Docker** (no build), or a **compressed file**.
  - Docker images: `local::list()` runs `docker image ls` and `docker ps` (names, sizes, ports,
    architecture, and the Compose project folder label; never environment variables or volumes).
    `scan::scan_image` can run Trivy (as a `aquasec/trivy` container) on the chosen image first (secrets and HIGH/CRITICAL only).
    Images that are not `linux/amd64` are shown but cannot be chosen, because the destinations run
    x86. Choosing an image writes `skyship.toml` with an `image` field (see below), in a folder
    the app owns (`<config>/apps/<name>`), since there is no project folder.
  - Compressed file: `.zip`, `.tar`, `.tar.gz`/`.tgz` are opened by `archive::extract` into a
    private temporary folder with limits (1 GiB, 20 000 files), rejecting paths that escape the
    folder and dropping symlinks. `.rar` goes through `bsdtar` if it is installed. Leftovers
    (`skyship-archive-*` in the temp folder) are removed at the next start.
- Permissions live in a settings file kept by the Rust side (`<config>/settings.json`), not in the
  webview: `read_profiles` (default on), `docker_list` (asked the first time the Docker source
  is chosen), `scan_images` (off by default) and `delete_unmanaged` (off by default). The commands `profiles` and `list_local_docker` check them themselves. A settings
  screen (gear menu) turns each one on or off.
- A terms-of-use screen comes first. Nothing is read from disk and no cloud call is made before
  the terms of the current version (`TERMS_VERSION` in `ui/app.js`) are accepted. The same screen
  holds the permission to list the AWS profile names (on by default). The acceptance is kept in the
  webview's local storage; the permissions are kept by the Rust side (see below). All of it can be
  reviewed from the gear menu.
- Three items in the sidebar, each with an icon and no numbering: **Apps** (the apps in the account),
  **New deploy** (pick the app and destination) and **Credentials** (the AWS account). Once an
  account is connected the app always starts on Apps, even when the list is empty. The deploy
  progress screen is not an item: it opens by itself when you publish.
  While a deploy runs the other items are locked (cancel is the way out); once it fails or is
  cancelled you can retry or go back, and when it succeeds the app goes to the live apps. The
  "technical details" link on the live screen reopens the last deploy's log. Items other than the account can be opened
  once the account is connected, even when they have nothing to show yet.
- The apps list marks the apps created by skyship (the `skyship:name` tag, `AppSummary.managed`).
  The Remove button is disabled for the others unless the setting "allow deleting apps skyship did
  not create" (`delete_unmanaged`, off by default) is on; the backend enforces it too.
- The apps list reloads itself every 10 seconds while an app is in a transitional state
  (`PENDING`, `DEPLOYING`, `UPDATING`, …) and the Apps screen is open, for up to 20 minutes; a
  failed reload keeps the list on screen. The Refresh button reloads on demand.
- Publishing again with the same name, destination and region **updates** the app: Lightsail reuses
  the service and creates a new deployment; EC2 reuses the instance (found by its `skyship:name`
  tag) and swaps the container, with a few seconds of downtime. The size (`power` or instance type)
  is only applied when the app is created. On the new-deploy screen the app list of the chosen
  region (`list_apps`, which carries each app's `size`) is checked as the name, destination and
  region change: when a match exists the button reads "Update <name>" and a note says what will be
  replaced, plus a warning when the chosen size differs from the one in use. If the list cannot be
  read, the screen treats the app as new.
- The profile menu in the sidebar footer (where the connected account is shown) lists the profiles
  found in `~/.aws/credentials` and `~/.aws/config` (names only, through `login::list_profiles`)
  and switches between them. A profile is checked before it replaces the current one; if the check
  fails, the error shows in the menu and nothing changes.
- At startup the app only uses the profile chosen last time, or one named `skyship` (the profile
  the app itself creates). Any other profile is used only when you choose it in the menu.
- Once an account is ready, the app looks for apps in the last used region (and in `us-east-1`).
  If it finds any, it opens the live apps screen directly; otherwise it opens the project screen.
- The live screen lists the apps of the chosen region through `list_apps` (all Lightsail container
  services, plus the EC2 instances tagged `skyship:name`). A destination that fails, for example
  for lack of permission, becomes a warning and does not stop the other one.
- External links (the IAM console, an app's address) are marked with an icon, and copy buttons show
  a small "Copied" balloon. The interface is available in Portuguese, English and Spanish, in a light and a dark
  theme. Both choices are kept in the webview's local storage.
- The user-facing texts live in `ui/i18n.js`. The three languages must keep exactly the same keys.
- On Linux the app sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup (unless it is already set).
  Without it, WebKitGTK on Wayland with an NVIDIA GPU closes the window right after opening with
  `Error 71 (Protocol error) dispatching to Wayland display`.

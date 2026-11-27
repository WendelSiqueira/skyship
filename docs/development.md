# Development

## Build and test

```bash
cargo build                       # the CLI ends up in target/debug/skyship
cargo test                        # unit tests of the core
cargo install cargo-audit --locked   # once
cargo audit                       # dependency advisories
```

The AWS SDK compiles many crates, which uses a lot of CPU, memory and disk IO. The repository
already limits parallelism and shrinks the artifacts:

- `.cargo/config.toml`: `jobs = 4`
- `Cargo.toml` (workspace root): `[profile.dev] debug = 0`

To build without disturbing the rest of the system:

```bash
nice -n 19 ionice -c3 cargo build
```

## The desktop app

`crates/gui` is left out of the default build (`default-members` in the root `Cargo.toml`), because
it pulls in Tauri and WebKit, which take a long time to compile. Build and run it on purpose:

```bash
nice -n 19 ionice -c3 cargo build -p skyship-gui
./target/debug/skyship-gui
```

On Linux it needs the WebKitGTK 4.1 development libraries (see the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).

There is no Node and no bundler: the frontend is the files in `crates/gui/ui`, embedded at compile
time. To look at the screens without Tauri, open `index.html` in a browser with a script that
defines a fake `window.__TAURI__` (`core.invoke`, `event.listen`, `dialog.open`, `opener.openUrl`)
before `app.js`, then call `showScreen(...)` and `onDeployEvent(...)` from the console.

## Trying it without a real AWS account

The AWS SDK honours `AWS_ENDPOINT_URL`, so the core can talk to a local AWS emulator. What was
checked with [moto](https://github.com/getmoto/moto) 5.2.6 (`pip install 'moto[server]'`, then
`python -m moto.server -p 5057`):

```bash
export AWS_ENDPOINT_URL=http://127.0.0.1:5057 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test
skyship doctor      # works: moto answers STS with a fake account
skyship status      # works for the EC2 target
```

Limits: moto has **no Lightsail** (the Lightsail target fails with an HTML error), `docker push`
needs a real registry (moto's ECR does not speak the Docker registry protocol), and SSM commands
are accepted but never run. So the deploy flows cannot be validated this way, and a mock written
by us cannot confirm the guesses listed under [Known gaps](#known-gaps) (the Lightsail registry
path and the required permissions). Those need a real account.

## Continuous integration and releases

Everything is in `.github/`; actions are pinned by commit hash (Dependabot proposes the updates).

- **`ci.yml`**, on every pull request and on `main`:
  - `fmt`: `cargo fmt --all -- --check` (`rustfmt.toml` sets 120 columns)
  - `core`: `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`, `cargo build --locked`
  - `gui`: the same for `skyship-gui` (installs the WebKitGTK 4.1 development packages)
  - `ui`: `node scripts/check-ui.mjs` (syntax of the frontend, the three languages have the same
    texts, every text key and element id used actually exists)
- **`audit.yml`**: `cargo audit`, when the dependencies change, on `main` and every Monday.
  Vulnerabilities fail it; the informational warnings from Tauri/GTK
  (see [security.md](security.md)) do not.
  - `windows`: `cargo test --locked --workspace` on Windows (it is not a required check yet)
- **`release.yml`**: builds the packages for users and, on a tag, creates a **draft** release.
  - **Packages:** on Linux an AppImage and a `.deb` (Ubuntu 22.04); on Windows an NSIS installer
    (`skyship_X.Y.Z_x64-setup.exe`, per user, no administrator needed) and a `-portable.zip` with
    `skyship.exe` (terminal) and `skyship-gui.exe`. The desktop app is the product; the zip is for
    running from a terminal and seeing the logs.
  - **Rehearsal:** a pull request that touches `release.yml`, `tauri.conf.json` or the icons, or a
    manual run, builds the same packages and keeps them as workflow artifacts for 7 days. Nothing is
    tagged, attested or released. Download them from the run page and try them.
  - **Tag `vX.Y.Z`:** the `check` job requires the tag to equal the workspace version (and the
    version in `crates/gui/tauri.conf.json`) and its commit to be on `main`; `cargo audit` and the
    tests must pass; the packages get a build provenance attestation signed by GitHub; and a draft
    release is created with `SHA256SUMS`. A `0.x` version is marked as a pre-release. Publishing the
    draft is a manual step. Check a download with `sha256sum -c SHA256SUMS` and
    `gh attestation verify <file> --repo WendelSiqueira/skyship`.
  - **Not signed yet:** the Windows installer is not code-signed, so SmartScreen warns ("More
    info → Run anyway"). macOS is not built yet.
  - `tauri-cli` is downloaded from the official releases with a pinned version and SHA-256
    (`TAURI_CLI_*` in the workflow); update both together.

To make a release: raise `version` in the root `Cargo.toml` **and** in `crates/gui/tauri.conf.json`
through a pull request, merge it, tag that commit on `main` (`git tag vX.Y.Z && git push origin
vX.Y.Z`), then review and publish the draft.

Run the same checks locally before opening a pull request:

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
cargo clippy --all-targets -p skyship-gui -- -D warnings && cargo test -p skyship-gui
node scripts/check-ui.mjs
```

## Testing on Windows

The desktop app has only been run on Linux so far. To try it on Windows:

- **Prerequisites:** [Rust](https://rustup.rs/) with the MSVC toolchain (it asks for the Visual Studio
  Build Tools, workload "Desktop development with C++"), the WebView2 runtime (already in Windows 11
  and in up-to-date Windows 10), Git, and [Docker Desktop](https://docs.docker.com/desktop/setup/install/windows-install/)
  with Linux containers.
- **Build and run from a terminal** (PowerShell), which also shows the logs:

  ```powershell
  cargo test --workspace                 # the same tests as CI
  cargo build -p skyship-gui             # the app
  .\target\debug\skyship-gui.exe
  cargo run -p skyship -- doctor         # the CLI
  ```

- **Or** download the installer or the `-portable.zip` from the rehearsal run of `release.yml`.
- **What has not been verified on Windows** (try these and report what breaks):
  - the AWS profiles are read from `%USERPROFILE%\.aws\credentials` and `config`; `skyship login` writes
    the credentials file there (no `600` mode on Windows; it relies on the profile folder's permissions)
  - choosing a project folder and building it with `docker build` (paths, long paths, `\r\n` in the
    Dockerfile)
  - the Trivy check mounts `/var/run/docker.sock` into the Trivy container; Docker Desktop may or
    may not accept that on Windows
  - opening a `.zip` / `.tar.gz` archive, and a `.rar` (it uses the `tar.exe` that comes with Windows)
  - the "Open" buttons (the default browser), the copy buttons, the three languages, light and dark
    themes, and uninstalling from "Apps & features"

## Licenses

skyship is `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`, `license` in the workspace
`Cargo.toml`). The dependencies are permissive (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode and
similar); there is no GPL or AGPL crate. The Tauri stack brings a few MPL-2.0 crates (`cssparser`,
`selectors`, `dtoa-short`, `option-ext`), used unmodified. Check again after changing dependencies
with `cargo metadata --format-version 1 | jq -r '.packages[] | "\(.name) \(.license)"'`.

## Rules to keep

- **The core never prints or asks.** Progress goes out as `progress::Event`.
- **Treat `skyship.toml` as untrusted.** Any new field that ends up in a command, a name or a remote
  script needs validation in `Config::validate`, with a test.
- **New AWS regions** are added to `crates/core/src/regions.rs`.
- **New `aws-sdk-*` crates** must use `default-features = false` (see [security.md](security.md)).
- Run `cargo audit` after changing dependencies.
- Texts shown to the user go in `crates/gui/ui/i18n.js`, in all three languages
  (`node scripts/check-ui.mjs` fails otherwise).
- When the terms of use change, update `TERMS.md`, `TERMS.pt-BR.md` and the `t.s*` texts in
  `i18n.js`, and bump `TERMS_VERSION` in `ui/app.js` so the app asks for acceptance again.

## Known gaps

- **Lightsail image path.** The push target is `{registry}/sr:<unique tag>`, the same as AWS's
  `lightsailctl` (the registry login is only valid for that repository). The push,
  `RegisterContainerImage` and the deployment with this path worked in one real deploy.
- **Only one kind of app has been deployed**: a small static site, from the desktop app, on both
  Lightsail and EC2 (a deploy and an update on each). Nothing else has been tried, and removing an
  EC2 app from the apps list was not exercised on a real account.
- Extra permissions AWS may require on Lightsail (for example IAM) are unverified beyond what that
  one deploy needed.
- The ECR lifecycle policy (keep the 3 most recent images) was written from the AWS documentation and has
  never run against a real account. ECR applies it asynchronously.
- `destroy` for EC2 does not remove the IAM role or the security group.
- The EC2 policy is limited to `skyship-*` roles, `skyship/*` repositories and tagged instances, but
  it was only exercised by one deploy and one update on a real account.
- An app published from an archive keeps its `skyship.toml` in the temporary extraction folder,
  which is removed at the next start; opening it again needs the file to be chosen again. Apps
  published from a local image keep their config in `<config>/apps/<name>`.
- `.rar` needs the system `bsdtar`, and its size limit is checked after extraction.
- The Trivy check was run for real through Docker on a public image and on an image holding a fake
  AWS key (`cargo test -p skyship-core scans_real_image -- --ignored --nocapture`, image chosen
  with `SKYSHIP_SCAN_IMAGE`), plus a fake backend for the screens. For a folder or archive the scan builds a temporary tag first (`scan::scan_project`); that path
  was tested with a fake backend only.
- Publishing a local image has not been run against a real account.
- The desktop app was opened on a real window (first screen only). The other screens were checked
  with a fake backend, and no deploy has been run through it.
- No support for environment variables, custom domains / HTTPS, or several containers.
- `skyship regions | head` prints a "Broken pipe" error (Rust's `println!` panics on a closed pipe).
- The `lightsail` column of the region list is unverified.

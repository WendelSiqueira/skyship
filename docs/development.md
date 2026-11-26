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

## Windows

Tested on Windows 11 with Docker Desktop. What it needs:

- Rust with the MSVC toolchain and the C++ build tools, for example from PowerShell:

  ```powershell
  winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  winget install --id Rustlang.Rustup -e
  # open a new terminal, then:
  rustup default stable-msvc
  ```

- Docker Desktop with the WSL 2 backend, in **Linux containers** mode. skyship runs the `docker`
  CLI, which reaches the Docker Desktop engine through its named pipe on its own; `DOCKER_HOST` and
  `docker context` are respected. Windows containers mode does not work (images are built for
  `linux/amd64`, and the Trivy check mounts `/var/run/docker.sock`, which only exists in the Linux
  VM). A Docker engine installed only inside a WSL distro is not visible to a native `skyship.exe`.
- WebView2 for the desktop app (it ships with Windows 11).

`nice`/`ionice` do not exist there; `cargo build` as is. The same commands under
[Build and test](#build-and-test) and [The desktop app](#the-desktop-app) apply, with
`.\target\debug\skyship-gui.exe`.

Things specific to Windows in the code:

- `~/.aws` is `%USERPROFILE%\.aws` when `HOME` is not set (`login::home_dir`).
- There is no file mode: the credentials file gets the ACL of the user profile folder, which is
  private by default.
- Paths go through `config::canonicalize`, which drops the `\\?\` prefix that `canonicalize` adds
  on Windows, so messages and `docker build` get plain `C:\...` paths.
- In a release build of the desktop app (`windows_subsystem = "windows"`), check that no console
  window flashes when it runs `docker`; this has not been checked yet.
- `.rar` needs `bsdtar`, which Windows ships as `tar.exe`, so `.rar` does not open there yet.
- Git with `core.autocrlf=true` checks out shell scripts with CRLF; copied into a Linux image they
  fail with `/bin/sh^M: not found`. Use a `.gitattributes` with `*.sh text eol=lf` in the project.

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
- **`release.yml`**: runs when a tag `vX.Y.Z` is pushed. It checks that the tag equals the workspace
  version and that its commit is on `main`, runs the tests, builds `skyship` and `skyship-gui` in
  release mode on Ubuntu 22.04, and creates a **draft** release with
  `skyship-vX.Y.Z-x86_64-linux.tar.gz` and its `.sha256` (both programs, the licenses, the READMEs
  and the terms). Publishing the draft is a manual step. The package carries a build provenance
attestation signed by GitHub (which workflow, which commit); check a download with
`gh attestation verify skyship-vX.Y.Z-x86_64-linux.tar.gz --repo WendelSiqueira/skyship`. The
binaries themselves are not code-signed.

To make a release: raise `version` in the root `Cargo.toml` through a pull request, merge it, tag
that commit on `main` (`git tag vX.Y.Z && git push origin vX.Y.Z`), then review and publish the draft.

Run the same checks locally before opening a pull request:

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
cargo clippy --all-targets -p skyship-gui -- -D warnings && cargo test -p skyship-gui
node scripts/check-ui.mjs
```

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

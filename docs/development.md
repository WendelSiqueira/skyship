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
- Texts shown to the user go in `crates/gui/ui/i18n.js`, in all three languages.
- When the terms of use change, update `TERMS.md`, `TERMS.pt-BR.md` and the `t.s*` texts in
  `i18n.js`, and bump `TERMS_VERSION` in `ui/app.js` so the app asks for acceptance again.

## Known gaps

- **Lightsail image path.** The push target is `{registry}/sr:<unique tag>`, the same as AWS's
  `lightsailctl` (the registry login is only valid for that repository). The push,
  `RegisterContainerImage` and the deployment with this path worked in one real deploy.
- **Only one full deploy has been completed**: Lightsail, from the desktop app, with a small static
  site. EC2 has never been run, and nothing else has been tried on Lightsail.
- Extra permissions AWS may require on Lightsail (for example IAM) are unverified beyond what that
  one deploy needed.
- ECR has no lifecycle policy, so images pile up (EC2 only).
- `destroy` for EC2 does not remove the IAM role or the security group.
- The EC2 IAM policy is broader than it should be.
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

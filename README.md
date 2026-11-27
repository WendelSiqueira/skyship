# skyship

**Put your Docker app online on the cloud, without needing to know the cloud.**

🇧🇷 [Leia em português](README.pt-BR.md)

skyship takes a folder with a `Dockerfile`, builds it on your computer, sends it to the cloud and
gives you back a public address. It is made for people who are not used to cloud services: it
explains each step and can remove everything when you are done, so the bill stops.

## How it works

1. **Connect your cloud account.** skyship tells you exactly which permissions to give it.
2. **Pick your app.** A project folder with a `Dockerfile` or, in the desktop app, a compressed file
   (`.zip`, `.tar`, `.tar.gz`, or `.rar` if `bsdtar` is installed) with one inside, or an image that
   already exists in your local Docker.
3. **Publish.** skyship builds, uploads and starts your app.
4. **Open the address** it gives you. To update the app, publish again with the same name. When you
   no longer need it, remove it (`skyship destroy`, or the Remove button in the desktop app).

It publishes to **AWS** (Amazon Web Services), in two ways:

| Destination | Good for |
|---|---|
| **Lightsail Containers** (default) | Small sites and apps. Simple, fixed price. |
| **EC2** | When you need more control over the machine. |

## Quick start

You need: [Docker](https://docs.docker.com/get-docker/) running, an AWS account, and the
[Rust toolchain](https://rustup.rs/) to build it. There are no prebuilt releases. It was tried on Linux
and on Windows 11 (see below).

```bash
git clone <this repository>
cd skyship
cargo build --release          # the program ends up in target/release/skyship

cd /path/to/your/project       # the folder with your Dockerfile
skyship policy                 # prints the permissions to give skyship (see below)
skyship login                  # asks for your access keys, checks them, saves them
skyship init --name myapp --target lightsail --port 80
skyship deploy
skyship status
skyship destroy --yes          # removes the app and stops the billing
```

The `--port` is the port your app listens on inside the container. Your app must answer with a
success code on `/`, because that is how Lightsail checks that it is alive.

There is also a desktop app, in Portuguese, English and Spanish, with a light and a dark theme. It
lists the apps in your account, publishes new ones, updates existing ones and removes them. It can
suggest what to publish from your local Docker and, if you turn it on, check the image with
[Trivy](https://trivy.dev) (run as a Docker container) for secrets and serious vulnerabilities
before publishing. What it may do on your computer is chosen in its Settings.

### Installing the programs

There is nothing to install besides the programs themselves: build them, then run them from where they
are or copy them wherever you like.

- **Command line:** `cargo build --release` leaves `skyship` (`skyship.exe`) in `target/release/`.
  `cargo install --path crates/cli` puts it in your Cargo `bin` folder instead.
- **Desktop app:** it is not part of the default build. Before building it, install what it needs:
  on Linux the WebKitGTK 4.1 development libraries (`webkit2gtk-4.1` on Arch,
  `libwebkit2gtk-4.1-dev` on Debian and Ubuntu; see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)), on Windows the Visual Studio
  Build Tools. Then `cargo build --release -p skyship-gui` and run `target/release/skyship-gui`
  (more in [docs/development.md](docs/development.md)).
- To remove everything, see [Uninstall](#uninstall).

### Giving skyship access to your account

Please do **not** use your main AWS login. Create a separate user that can only do what skyship
needs. Step by step instructions are in [docs/permissions.md](docs/permissions.md).

## What works today

| | Status |
|---|---|
| `init`, `policy`, `regions` | Work. |
| `doctor` | Works. |
| `login` | Written and its file handling is tested, but it has not been used interactively on a real account yet. |
| `status` | Works against Lightsail and EC2. |
| `deploy` to Lightsail | Works, including updating an app that already exists. Tested from the desktop app (new accounts can have a limit of zero container services; you may need to ask AWS to raise it). |
| `deploy` to EC2 | Works, including updating an app. Tested from the desktop app. The address is plain `http://`: the EC2 destination has no HTTPS. |
| Desktop app | Works on AWS: deploying, updating and removing apps on Lightsail and EC2, the apps list and the skyship tag, the Trivy check, and publishing from a project folder, a local image or an archive. |

## Uninstall

1. **Remove your apps first.** Uninstalling skyship does **not** remove what it published: an app
   keeps running (and billing) until you remove it. Use **Remove** in the desktop app (Apps), or
   `skyship destroy --yes` in the app's folder. When you remove an app, skyship also deletes what it
   created for it (on EC2: the security group, the ECR repository with its images, and the IAM role and
   profile).
2. **Remove the program.** It was built from source, so delete the folder you cloned (the programs are
   in its `target/` folder). If you ran `cargo install --path crates/cli`, run `cargo uninstall skyship`.
3. **Remove what skyship saved on your computer.** Nothing is saved anywhere else.

   | What | Where | Notes |
   |---|---|---|
   | AWS access keys (the profile `skyship`, if you used `skyship login` or the app) | `~/.aws/credentials` (Windows: `%USERPROFILE%\.aws\credentials`) | Delete the `[skyship]` block. **Also delete the access key in AWS** (IAM → Users → your user → Security credentials): removing the file does not cancel it. |
   | The desktop app's settings, the terms you accepted, and the configuration of apps published from a local image | Linux: `~/.config/dev.skyship.gui` and `~/.local/share/dev.skyship.gui`. Windows: `%APPDATA%\dev.skyship.gui` and `%LOCALAPPDATA%\dev.skyship.gui` | Delete the folders. |
   | Archives you opened in the app | `skyship-archive-*` in the temporary folder | Removed on the next start; safe to delete. |
   | Docker leftovers | The local images `skyship-build/*` (one per Lightsail deploy) and, if you used the Trivy check, the image `aquasec/trivy:0.58.0` and the volume `skyship-trivy-cache` | `docker image rm $(docker image ls 'skyship-build/*' -q)` and `docker volume rm skyship-trivy-cache` |

## Questions

**Do I need the AWS command line tool?** No.

**Is it safe to give it my keys?** They are checked with AWS before they are used, and **you choose** what
happens next: save them on your computer, in `~/.aws/credentials` (restricted permissions on Linux and
macOS, your user folder's permissions on Windows; any program running as you can read that file), or use
them only now, kept in the app's memory and gone when you close it (desktop app only; nothing is
written to the computer). The app does not delete saved keys later: to remove them, delete the `[skyship]` profile from that file and the access key in AWS (IAM → Users → Security credentials). Use the dedicated user from
[docs/permissions.md](docs/permissions.md), never your main account. More in
[docs/security.md](docs/security.md).

**How do I update an app?** Publish again with the same name, destination and region. The desktop
app then shows "Update <name>". Lightsail swaps the version; EC2 swaps the container, with a few
seconds of downtime. The size is only applied when the app is created.

**Why is "Remove" disabled for an app in the list?** The list shows every Lightsail container
service in the region, but only removes the ones that carry the `skyship:name` tag that skyship puts
on what it creates. For the others, check the option in Settings to allow it.

**I got "maximum limit of Lightsail Container Services".** Your account has no room for another
container service. Delete one you do not use, or ask AWS to raise the limit (Service Quotas, or a
support request). This is a limit of the account, not of skyship.

**Will I be charged after I try it?** Yes, as long as the app exists. `skyship destroy --yes`
(or Remove in the desktop app) removes the Lightsail service. For EC2 it ends the instance and also deletes what skyship created for it
(the security group, the ECR repository with its images, and the IAM role and profile).

**Does it work on Windows or macOS?** On Windows, yes: it was built and tested on Windows 11 with
Docker Desktop (WSL 2 backend, Linux containers mode), and a site was published to Lightsail from the
desktop app. Building needs Rust with the MSVC toolchain and the Visual Studio Build Tools (see
[docs/development.md](docs/development.md#windows)). macOS has not been tried.

## Terms of use

Using skyship means accepting the [terms of use](TERMS.md): you are responsible for the costs of
what it creates in your account, and it is provided without warranty. The desktop app asks for your
acceptance the first time it opens.

## Not affiliated with AWS

skyship is an independent open source project. It has **no relationship with, sponsorship from, or
approval by** Amazon Web Services. "AWS", "Amazon Lightsail", "Amazon EC2" and other names are
trademarks of their owners and appear here only to say which services the tool talks to.

## For contributors

- [docs/architecture.md](docs/architecture.md): how the code is organised and how each destination works
- [docs/security.md](docs/security.md): what the tool protects against and what is still open
- [docs/permissions.md](docs/permissions.md): the exact permissions and how to create the user
- [docs/development.md](docs/development.md): building, testing, and the list of known gaps

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at
your option. Both allow using, changing and sharing skyship, including commercially, as long as the
notice is kept; neither comes with any warranty.

Unless you explicitly state otherwise, any contribution you submit for inclusion in skyship, as
defined in the Apache-2.0 license, is dual licensed as above, without any additional terms or
conditions.

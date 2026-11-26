# skyship

**Put your Docker app online on the cloud, without needing to know the cloud.**

🇧🇷 [Leia em português](README.pt-BR.md)

skyship takes a folder with a `Dockerfile`, builds it on your computer, sends it to the cloud and
gives you back a public address. It is made for people who are not used to cloud services: it
explains each step and can remove everything when you are done, so the bill stops.

## How it works

1. **Connect your cloud account.** skyship tells you exactly which permissions to give it.
2. **Pick your app.** A project folder with a `Dockerfile`, a compressed file (`.zip`, `.tar.gz`)
   with one inside, or (in the desktop app) an image that already exists in your local Docker.
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
[Rust toolchain](https://rustup.rs/) to build it. There are no prebuilt releases, and it has only
been tried on Linux.

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
before publishing. What it may do on your computer is chosen in its Settings. It is not part of the
default build: run
`cargo build -p skyship-gui` and then `./target/debug/skyship-gui` (more in
[docs/development.md](docs/development.md)).

### Giving skyship access to your account

Please do **not** use your main AWS login. Create a separate user that can only do what skyship
needs. Step by step instructions are in [docs/permissions.md](docs/permissions.md).

## What works today

| | Status |
|---|---|
| `init`, `policy`, `regions` | Work. |
| `doctor` | Works. Used with a real account. |
| `login` | Written and its file handling is tested, but it has not been used interactively on a real account yet. |
| `status` | Works against Lightsail. |
| `deploy` to Lightsail | Works from the desktop app with a small static site, including updating an app that already exists (new accounts can have a limit of zero container services; you may need to ask AWS to raise it). Not tried with other apps. |
| `deploy` to EC2 | Works from the desktop app with a small static site (a deploy and an update). The address is plain `http://`: the EC2 destination has no HTTPS. Not tried with other apps. |
| Desktop app | Builds and runs. Deploying and updating a small static site on Lightsail and on EC2, the apps list and the skyship tag were done on a real account. Removing an app from the list, the Trivy check, and publishing from a local image or an archive were only checked with a fake backend. |

## Questions

**Do I need the AWS command line tool?** No.

**Is it safe to give it my keys?** They stay on your computer, in `~/.aws/credentials` with
restricted permissions, and are checked with AWS before being saved. Use the dedicated user from
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
(or Remove in the desktop app) removes the Lightsail service. For EC2 it ends the instance, but the permissions role and network
rule it created stay behind (they are free).

**Does it work on Windows or macOS?** It has not been tried there.

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

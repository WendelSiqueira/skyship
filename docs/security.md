# Security

skyship handles cloud credentials and runs builds and remote commands, so it is written to treat
its inputs carefully. This page says what is protected and what is still open.

## `skyship.toml` is untrusted input

It can come from someone else's project. `Config::load_from` (and `deploy`, `status` and `destroy`
again, for safety) validate it:

- **`region`** must be in the closed list in `crates/core/src/regions.rs` (`skyship regions` prints
  it). The region ends up inside a script that runs as root on the EC2 instance, so nothing outside
  the list gets through. A new AWS region needs a new line in that file. The `lightsail` column of
  that list was written from memory and is not verified; it is informational and never blocks a
  deploy.
- **`port`, `platform`, `ec2.instance_type`, `lightsail.power`, `lightsail.scale`, `name`** follow
  strict formats.
- **`context` and `dockerfile`** must be relative paths that stay inside the project folder.
  Absolute paths, `..` that escapes, and symlinks pointing outside are rejected, so a foreign
  `skyship.toml` cannot make the build pack personal files (for example `~/.aws`) into the image
  that gets uploaded.

- **`image`** (publish a local image) must look like a Docker image reference: no leading `-`, no
  spaces or shell characters. It is only checked for syntax: a foreign `skyship.toml` can name any
  image that exists on your computer, and `deploy` would upload it to **your own** cloud account.
  Read a `skyship.toml` you did not write before deploying. The image is checked to be
  `linux/amd64` (the platform of the destinations) before it is sent.

The tests are in `crates/core/src/config.rs`.

## Local Docker and compressed files

- Listing the local Docker is off until you allow it (the app asks the first time you pick that
  source) and can be turned off in the settings. The backend refuses the call when it is off. Only
  names, sizes, ports, architecture and the Compose folder label are read. Environment variables,
  volumes and container contents are not.
- An image is sent **as it is**. Images can have passwords or keys baked in; the app warns about it.
- Optional check with [Trivy](https://trivy.dev) (off by default; a checkbox on the project screen
  and a switch in the settings; the backend refuses `scan_image` while it is off). For the image you chose
  or, for a project folder or archive, for the image built from it (tagged `skyship-scan/<name>:check`
  and removed afterwards; the deploy then builds again, reusing the layer cache). Nothing is installed: Trivy runs as a
  container (`aquasec/trivy`, pinned to a version in `scan::TRIVY_IMAGE`). **That container gets the
  Docker socket, which is equivalent to full control of your Docker**, so the image is the official
  one and pinned; bump it on purpose. A volume `skyship-trivy-cache` keeps the vulnerability
  database. It runs `image --scanners vuln,secret --severity HIGH,CRITICAL` and reports secrets (rule and file
  only: the matched text is never read back) and the count of HIGH/CRITICAL vulnerabilities. It is
  a warning, not a block, and "nothing found" does not mean the image is safe. The first run
  downloads Trivy's image and database. The Docker socket path is the Linux one; other systems
  were not tried.
  A running container's volumes, variables and networks do not travel with its image.
- A compressed file is treated as untrusted. `.zip`, `.tar` and `.tar.gz` are opened by the app
  itself, which rejects paths that leave the target folder (`..`, absolute paths), skips symlinks,
  hard links and special files, and stops at 1 GiB or 20 000 files, counting the bytes actually
  written. `.rar` is opened by the system `bsdtar` (a limit of 120 s, and the result is checked
  afterwards), so its protection against oversized archives is weaker: it can write the data before
  the size check removes it. The files go to a private temporary folder (mode `0700`).

## Removing apps from the list

The apps list shows every Lightsail container service in the region, and only the EC2 instances that
carry the `skyship:name` tag. The deploy tags a Lightsail service with the same tag (created or
updated by skyship; services created before the tag existed get it on their next update, if the user
has `lightsail:TagResource`). Removing a service **without** the tag is refused by the backend
(`destroy_app` returns `unmanaged`) unless the setting "allow deleting apps skyship did not create"
is on. It is off by default and turning it on asks for confirmation; the list then removes such a
service after a stronger warning. The tag proves nothing about who owns a service (anyone with
access to the account can add it): it is only a guard against removing something by mistake.

## Credentials

- The desktop app reads the names of the AWS profiles (never the keys) only after the terms of use
  are accepted, and only if the permission to list profiles is on. A profile's keys are used by
  the AWS SDK only when that profile is chosen. The one exception at startup is the profile chosen
  last time and a profile named `skyship`, which the app itself creates.
- `skyship login` checks the keys with AWS (`sts:GetCallerIdentity`) **before** saving anything,
  then writes them to `~/.aws/credentials` through a temporary file with permission `600`.
- The keys sit in plain text in that file. Any program running as your user can read them.
- The permissions you give the user are the real limit. Use the minimal policies in
  [permissions.md](permissions.md), never the root account.

## Dependencies

`cargo audit` reports **no vulnerabilities** at the time of the last run (576 crates, with the
desktop app). It does print 6 informational warnings, all coming from Tauri and the GTK bindings it
uses: `chrono`, `proc-macro-error`, `zbus_names`, `zvariant` and `zvariant_derive` are unmaintained,
and `glib 0.18` has an `unsound` advisory about `VariantStrIter`. They cannot be fixed from this
repository without replacing Tauri. The `aws-sdk-*` crates in
`crates/core/Cargo.toml` use `default-features = false` with `default-https-client`: their default
turns on a legacy HTTP client (`hyper 0.14`, `rustls 0.21`, `h2 0.3`) that carried 7 advisories and
is not used. **When adding another `aws-sdk-*` crate, repeat that setting**, or the old stack comes
back. `Cargo.lock` is versioned on purpose (the project builds binaries).

## Still open

- AWS keys in plain text in `~/.aws/credentials`.
- `docker` is found through `PATH`. A fake `docker` earlier in the `PATH` would receive the registry
  password. Exploiting this needs control of your environment, so the priority is low.
- The EC2 policy still uses `Resource: "*"` where AWS cannot limit by resource (`RunInstances`,
  security groups, the read calls) and for the ECR login token. IAM, ECR repositories and
  starting/terminating instances are limited to what skyship creates.
- The desktop app's content security policy allows inline styles (`style-src 'unsafe-inline'`).
  Scripts are limited to the app's own files. The access keys typed in the app go through the
  webview to the Rust side once, and are not stored by the frontend.

Found a problem? Please open an issue, or contact the maintainers privately if it is sensitive.

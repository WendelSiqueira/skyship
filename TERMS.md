# Terms of use

🇧🇷 [Leia em português](TERMS.pt-BR.md)

These are the terms shown in the skyship desktop app, which asks you to accept them the first time
you open it (and again whenever they change). Version `2026-11-26-5`.

## What skyship is

skyship is an independent open source project. It has no relationship with, sponsorship from, or
approval by Amazon Web Services (AWS) or Docker. Those names are trademarks of their owners.

## What it does in your account

skyship runs on your computer. It builds your project with Docker, uploads it to your cloud account
and creates resources there (for example a Lightsail service, an EC2 instance, an ECR repository,
IAM roles and network rules), using the credentials you provide. If you allow it, it also lists the
Docker images and containers on this computer (names, sizes and ports) to suggest what to publish,
it can open a compressed file that you choose, in a temporary folder, and, if you turn it on, it
runs Trivy (as a Docker container, with access to the Docker on this computer) on the image you chose or
that it builds before publishing it.

## Costs

Resources created in your account are billed to you by the cloud provider for as long as they exist.
skyship does not show prices. You are
responsible for checking prices, setting up budget alerts and removing what you no longer use.
Removing an app in skyship may not remove everything that was created.

## Your credentials

The keys you type are validated with AWS and saved in the AWS credentials file on this computer
(`~/.aws/credentials`). If you allow it, skyship reads the names of the profiles in the AWS
configuration files on this computer (`~/.aws/credentials` and `~/.aws/config`) so you can choose
one, and uses the profile you choose to act on your account. Use a dedicated user with only the
permissions skyship asks for. Never use the root account.

## No warranty

skyship is provided "as is", without warranty of any kind. It is an early version: a deploy may
fail, leave resources behind, or behave in ways that were not tested.

## Limitation of liability

To the extent permitted by law, the authors and contributors are not liable for any loss or damage
arising from the use of skyship, including cloud charges, data loss, service interruption, security
incidents or misconfiguration of your account. You use skyship at your own risk.

## Third-party terms

Your use of AWS, Docker and other services is governed by their own terms, and you are responsible
for following them.

## Changes

These terms may change. When they do, the app asks you to accept them again.

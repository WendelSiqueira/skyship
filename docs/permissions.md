# Permissions

skyship needs an AWS access key. Do not use your main login or the root account: create a separate
user that can only do what skyship needs. `skyship policy` prints the policy below for the
destination you choose (`--target lightsail` or `--target ec2`).

## Creating the user (AWS console)

1. **IAM → Policies → Create policy → JSON.** Paste the policy for your destination and name it
   `skyship-lightsail` (or `skyship-ec2`).
2. **IAM → Users → Create user**, for example `skyship-deployer`. Leave "console access" off.
   Attach the policy from step 1.
3. Open the user, go to **Security credentials → Create access key**, choose "Command Line
   Interface", and copy the *Access key ID* and the *Secret* (the secret is shown only once).
4. Run `skyship login` and paste them. The keys are checked with AWS before anything is saved.

Delete the access key (or the user) when you stop using skyship, and rotate it from time to time.

## Lightsail

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": [
      "lightsail:GetContainerServices",
      "lightsail:CreateContainerService",
      "lightsail:CreateContainerServiceRegistryLogin",
      "lightsail:RegisterContainerImage",
      "lightsail:CreateContainerServiceDeployment",
      "lightsail:DeleteContainerService",
      "lightsail:TagResource"
    ],
    "Resource": "*"
  }]
}
```

`DeleteContainerService` is only used by `destroy`. `TagResource` puts the `skyship:name` tag on the
service; without it a deploy still works, but the service is not shown as created by skyship. `sts:GetCallerIdentity`, which `doctor` and
`deploy` call, needs no permission.

Not verified: whether AWS asks for anything extra (for example IAM actions) when it creates the
service. If a deploy fails with `AccessDenied`, the message names the missing action.

## EC2

**Derived from reading the code and never tested.** Adjust it according to the `AccessDenied`
messages you get.

- **ECR:** `GetAuthorizationToken`, `DescribeRepositories`, `CreateRepository`,
  `BatchCheckLayerAvailability`, `InitiateLayerUpload`, `UploadLayerPart`, `CompleteLayerUpload`,
  `PutImage`
- **EC2:** `DescribeInstances`, `RunInstances`, `StartInstances`, `TerminateInstances`,
  `CreateTags`, `DescribeSecurityGroups`, `CreateSecurityGroup`, `AuthorizeSecurityGroupIngress`
- **IAM:** `GetRole`, `CreateRole`, `AttachRolePolicy`, `GetInstanceProfile`,
  `CreateInstanceProfile`, `AddRoleToInstanceProfile`, `PassRole`
- **SSM:** `GetParameter`, `DescribeInstanceInformation`, `SendCommand`, `GetCommandInvocation`

Be aware that `iam:CreateRole` and `iam:PassRole` with `Resource: "*"` is a broad grant (it allows
privilege escalation inside the account). Restricting them to roles named `skyship-*` is on the
[list of known gaps](development.md#known-gaps).

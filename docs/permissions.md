# Permissions

Em português: [permissions.pt-BR.md](permissions.pt-BR.md).

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

**Derived from reading the code and tested on a real account:** deploys and updates from the
desktop app worked with this policy. Other apps may need more. Adjust it according to the
`AccessDenied` messages you get.

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "EcrLogin",
      "Effect": "Allow",
      "Action": "ecr:GetAuthorizationToken",
      "Resource": "*"
    },
    {
      "Sid": "EcrSkyshipRepositories",
      "Effect": "Allow",
      "Action": [
        "ecr:DescribeRepositories", "ecr:CreateRepository", "ecr:DeleteRepository", "ecr:PutLifecyclePolicy", "ecr:BatchCheckLayerAvailability",
        "ecr:InitiateLayerUpload", "ecr:UploadLayerPart", "ecr:CompleteLayerUpload", "ecr:PutImage"
      ],
      "Resource": "arn:aws:ecr:*:*:repository/skyship/*"
    },
    {
      "Sid": "Ec2ReadAndCreate",
      "Effect": "Allow",
      "Action": [
        "ec2:DescribeInstances", "ec2:DescribeInstanceStatus", "ec2:DescribeSecurityGroups", "ec2:DeleteSecurityGroup", "ec2:RunInstances",
        "ec2:CreateSecurityGroup", "ec2:AuthorizeSecurityGroupIngress"
      ],
      "Resource": "*"
    },
    {
      "Sid": "Ec2TagOnLaunch",
      "Effect": "Allow",
      "Action": "ec2:CreateTags",
      "Resource": "arn:aws:ec2:*:*:instance/*",
      "Condition": { "StringEquals": { "ec2:CreateAction": "RunInstances" } }
    },
    {
      "Sid": "Ec2OnlySkyshipInstances",
      "Effect": "Allow",
      "Action": ["ec2:StartInstances", "ec2:TerminateInstances"],
      "Resource": "arn:aws:ec2:*:*:instance/*",
      "Condition": { "Null": { "ec2:ResourceTag/skyship:name": "false" } }
    },
    {
      "Sid": "IamSkyshipRoleAndProfile",
      "Effect": "Allow",
      "Action": [
        "iam:GetRole", "iam:CreateRole", "iam:GetInstanceProfile", "iam:ListAttachedRolePolicies",
        "iam:CreateInstanceProfile", "iam:AddRoleToInstanceProfile",
        "iam:RemoveRoleFromInstanceProfile", "iam:DeleteInstanceProfile", "iam:DeleteRole"
      ],
      "Resource": ["arn:aws:iam::*:role/skyship-*", "arn:aws:iam::*:instance-profile/skyship-*"]
    },
    {
      "Sid": "IamAttachOnlyTheTwoManagedPolicies",
      "Effect": "Allow",
      "Action": ["iam:AttachRolePolicy", "iam:DetachRolePolicy"],
      "Resource": "arn:aws:iam::*:role/skyship-*",
      "Condition": {
        "ArnEquals": {
          "iam:PolicyARN": [
            "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore",
            "arn:aws:iam::aws:policy/AmazonEC2ContainerRegistryReadOnly"
          ]
        }
      }
    },
    {
      "Sid": "PassTheRoleOnlyToEc2",
      "Effect": "Allow",
      "Action": "iam:PassRole",
      "Resource": "arn:aws:iam::*:role/skyship-*",
      "Condition": { "StringEquals": { "iam:PassedToService": "ec2.amazonaws.com" } }
    },
    {
      "Sid": "SsmAmiLookup",
      "Effect": "Allow",
      "Action": "ssm:GetParameter",
      "Resource": "arn:aws:ssm:*::parameter/aws/service/ami-amazon-linux-latest/*"
    },
    {
      "Sid": "SsmRunCommand",
      "Effect": "Allow",
      "Action": "ssm:SendCommand",
      "Resource": ["arn:aws:ssm:*:*:document/AWS-RunShellScript", "arn:aws:ec2:*:*:instance/*"]
    },
    {
      "Sid": "SsmRead",
      "Effect": "Allow",
      "Action": ["ssm:DescribeInstanceInformation", "ssm:GetCommandInvocation"],
      "Resource": "*"
    }
  ]
}
```

What each part allows, and why it is limited the way it is:

- **ECR:** only repositories named `skyship/*` (that is where skyship pushes the image); the login
  token (`GetAuthorizationToken`) cannot be limited to a resource.
- **IAM:** only roles and instance profiles named `skyship-*`. `AttachRolePolicy` only accepts the
  two AWS managed policies the instance needs (SSM and read-only ECR), and `PassRole` only passes
  the role to the EC2 service. This is what keeps `iam:PassRole` from being open to every role in
  the account.
- **EC2:** starting and terminating only works on instances that carry the `skyship:name` tag.
  `RunInstances`, creating the security group and the read calls (including `DescribeInstanceStatus`,
  used to show the instance status checks) cannot be limited by resource, so they stay on `*`.
- **SSM:** reading the public Amazon Linux AMI parameter, and running `AWS-RunShellScript` on the
  instance.

Create it as a **customer managed policy** (step 1 above), not as an inline policy on the user:
inline policies are limited to 2,048 characters and this one is about 2,200 without spaces.


## Why each permission

Every action in the policies above is used by one step of a deploy. Nothing is asked "just in case".

### Lightsail

Lightsail needs no IAM role: the service runs your container, and the image goes to the registry
that belongs to the service itself.

| Action | What skyship does with it |
|---|---|
| `lightsail:GetContainerServices` | Look for the service by name before creating it, wait until it is ready, read its state, address, size and tags, and fill the apps list. |
| `lightsail:CreateContainerService` | Create the service the first time an app is published. |
| `lightsail:CreateContainerServiceRegistryLogin` | Get temporary credentials for the service's registry, to `docker push` the image. |
| `lightsail:RegisterContainerImage` | Tell the service which pushed image to use. |
| `lightsail:CreateContainerServiceDeployment` | Start a new version of the app (the first deploy and every update). |
| `lightsail:TagResource` | Put the `skyship:name` tag on the service, so the apps list can tell what skyship created. Optional: a deploy works without it. |
| `lightsail:DeleteContainerService` | Remove the app (`destroy`, or Remove in the desktop app). |

### EC2

On EC2 skyship creates three things next to the instance. They exist because the instance has to
do things on its own, and because skyship does not use SSH:

| Created by skyship | What it is for |
|---|---|
| IAM role `skyship-<name>` | The identity **of the instance**, not yours. It carries only two AWS managed policies: `AmazonSSMManagedInstanceCore`, so the SSM agent can register and receive the deploy command (this is why there is no SSH, no key pair and no port 22), and `AmazonEC2ContainerRegistryReadOnly`, so the instance can pull the image from ECR (read only: it cannot push or delete anything). |
| Instance profile `skyship-<name>` | EC2 cannot take a role directly; the profile is the wrapper that attaches the role to the instance. It has the same name and holds only that role. |
| Security group `skyship-<name>` | The instance's firewall: only port 80, open to the internet. |
| ECR repository `skyship/<name>` | Where the image is stored so the instance can pull it. |

Your user needs these permissions to do that:

| Action | What skyship does with it |
|---|---|
| `ecr:GetAuthorizationToken` | Log Docker in to ECR. |
| `ecr:DescribeRepositories`, `ecr:CreateRepository`, `ecr:DeleteRepository` | Check whether the repository `skyship/<name>` exists, create it if not, and delete it (with its images) when the app is removed. |
| `ecr:PutLifecyclePolicy` | Set the repository's lifecycle policy: keep only the 3 most recent images and let ECR delete the older ones. Optional: without it a deploy works, but old images pile up. |
| `ecr:BatchCheckLayerAvailability`, `ecr:InitiateLayerUpload`, `ecr:UploadLayerPart`, `ecr:CompleteLayerUpload`, `ecr:PutImage` | The steps of `docker push`: which layers ECR already has, uploading each layer, and registering the image. |
| `ec2:DescribeInstances` | Find the instance by its tag, wait for it to be `running`, read its public address, and fill the apps list. |
| `ec2:DescribeInstanceStatus` | Read the status checks (`initializing`, `ok`) shown in the log and in the apps list. Optional. |
| `ec2:DescribeSecurityGroups`, `ec2:CreateSecurityGroup`, `ec2:AuthorizeSecurityGroupIngress`, `ec2:DeleteSecurityGroup` | Reuse or create the security group, open port 80, and delete the group when the app is removed. |
| `ec2:RunInstances` | Launch the instance the first time. |
| `ec2:CreateTags` | Tag the instance with `skyship:name` and `Name` at launch (only at launch). |
| `ec2:StartInstances` | Wake up a stopped instance when the app is published again. |
| `ec2:TerminateInstances` | Remove the app. |
| `iam:GetRole`, `iam:GetInstanceProfile`, `iam:ListAttachedRolePolicies` | Check whether the role and the profile already exist (to reuse them), and that the role is the one skyship created before deleting it. |
| `iam:CreateRole`, `iam:CreateInstanceProfile`, `iam:AddRoleToInstanceProfile` | Create them the first time. |
| `iam:AttachRolePolicy`, `iam:DetachRolePolicy` | Attach the two managed policies to the role, and detach them when the app is removed (and only those two, see above). |
| `iam:RemoveRoleFromInstanceProfile`, `iam:DeleteInstanceProfile`, `iam:DeleteRole` | Delete the instance profile and the role when the app is removed. |
| `iam:PassRole` | Hand the role to EC2 when launching the instance. Without it `RunInstances` is refused; it is limited to `skyship-*` roles and to the EC2 service. |
| `ssm:GetParameter` | Read the id of the current Amazon Linux image from its public parameter. |
| `ssm:DescribeInstanceInformation` | Wait until the instance's SSM agent is online. |
| `ssm:SendCommand`, `ssm:GetCommandInvocation` | Run the deploy script on the instance (log in to ECR, pull, restart the container) and read whether it worked. |

### What is removed when you remove an app

Removing an EC2 app terminates the instance and then deletes what skyship created for it: the
security group, the ECR repository `skyship/<name>` (with its images) and the IAM role and instance
profile `skyship-<name>`. Nothing it did not create is touched:

- the security group is only deleted if its description is the one skyship writes;
- the role is only deleted if its trust policy is exactly skyship's, it has no policy besides the
  two managed ones, and no instance of the same app exists in another region (the IAM role is
  global, shared by every region);
- IAM does not stop you from deleting a role that a running instance uses, so skyship is careful
  with it: the role is **kept** if the app's instance has not finished terminating, and also if it
  could not check one of the other regions (a network or permission error; only a region that the
  account has not enabled counts as empty). The log names the region it could not check, and
  removing the app again later finishes the job;
- anything that cannot be deleted is kept, and the log says why.

The cleanup also runs when there is no instance left (to clean up after an earlier removal): remove
the app again from the folder with `skyship destroy`.

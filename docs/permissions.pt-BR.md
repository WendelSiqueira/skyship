# Permissões

Em inglês: [permissions.md](permissions.md).

O skyship precisa de uma chave de acesso da AWS. Não use o seu login principal nem a conta raiz: crie
um usuário separado, que só pode fazer o que o skyship precisa. `skyship policy` imprime a política
abaixo para o destino escolhido (`--target lightsail` ou `--target ec2`).

## Criando o usuário (console da AWS)

1. **IAM → Policies → Create policy → JSON.** Cole a política do seu destino e dê o nome
   `skyship-lightsail` (ou `skyship-ec2`).
2. **IAM → Users → Create user**, por exemplo `skyship-deployer`. Deixe o "acesso ao console"
   desligado. Anexe a política do passo 1.
3. Abra o usuário, vá em **Security credentials → Create access key**, escolha "Command Line
   Interface" e copie o *Access key ID* e o *Secret* (o secret só aparece uma vez).
4. Rode `skyship login` e cole os dois. As chaves são conferidas na AWS antes de qualquer coisa ser
   salva.

Apague a chave de acesso (ou o usuário) quando parar de usar o skyship, e troque-a de tempos em
tempos.

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

`DeleteContainerService` só é usada pelo `destroy`. `TagResource` coloca a tag `skyship:name` no
serviço; sem ela o deploy funciona, mas o serviço não aparece como criado pelo skyship. O
`sts:GetCallerIdentity`, que o `doctor` e o `deploy` chamam, não precisa de permissão.

Não verificado: se a AWS pede algo a mais (por exemplo ações do IAM) ao criar o serviço. Se um deploy
falhar com `AccessDenied`, a mensagem diz qual ação falta.

## EC2

**Derivada da leitura do código e testada numa conta real:** deploys e atualizações pelo app de
desktop funcionaram com esta política. Outros apps podem precisar de
mais. Ajuste conforme as mensagens de `AccessDenied` que aparecerem.

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

O que cada parte permite, e por que ela é limitada assim:

- **ECR:** só repositórios chamados `skyship/*` (é onde o skyship envia a imagem). O token de login
  (`GetAuthorizationToken`) não pode ser limitado a um recurso.
- **IAM:** só roles e instance profiles chamados `skyship-*`. O `AttachRolePolicy` só aceita as duas
  políticas gerenciadas da AWS de que a instância precisa (SSM e leitura do ECR), e o `PassRole` só
  entrega a role ao serviço EC2. É isso que impede o `iam:PassRole` de valer para todas as roles da
  conta.
- **EC2:** iniciar e encerrar só funciona em instâncias que têm a tag `skyship:name`. O
  `RunInstances`, a criação do security group e as leituras (incluindo `DescribeInstanceStatus`,
  usada para mostrar os status checks da instância) não podem ser limitados por recurso, então ficam
  em `*`.
- **SSM:** ler o parâmetro público da AMI do Amazon Linux e executar o `AWS-RunShellScript` na
  instância.

Crie como **política gerenciada pelo cliente** (passo 1 acima), e não como política embutida no
usuário: as embutidas têm limite de 2.048 caracteres e esta tem cerca de 2.200 sem espaços.

## Por que cada permissão

Toda ação das políticas acima é usada por um passo do deploy. Nada é pedido "por via das dúvidas".

### Lightsail

O Lightsail não precisa de nenhuma role do IAM: o serviço roda o seu container, e a imagem vai para o
registry do próprio serviço.

| Ação | O que o skyship faz com ela |
|---|---|
| `lightsail:GetContainerServices` | Procurar o serviço pelo nome antes de criar, esperar até ele ficar pronto, ler o estado, o endereço, o tamanho e as tags, e montar a lista de apps. |
| `lightsail:CreateContainerService` | Criar o serviço na primeira vez que um app é publicado. |
| `lightsail:CreateContainerServiceRegistryLogin` | Obter credenciais temporárias do registry do serviço, para enviar a imagem com `docker push`. |
| `lightsail:RegisterContainerImage` | Dizer ao serviço qual imagem enviada ele deve usar. |
| `lightsail:CreateContainerServiceDeployment` | Iniciar uma nova versão do app (o primeiro deploy e cada atualização). |
| `lightsail:TagResource` | Colocar a tag `skyship:name` no serviço, para a lista de apps saber o que o skyship criou. Opcional: o deploy funciona sem ela. |
| `lightsail:DeleteContainerService` | Remover o app (`destroy`, ou Remover no app de desktop). |

### EC2

Na EC2 o skyship cria quatro coisas junto da instância. Elas existem porque a instância precisa fazer
algumas coisas sozinha, e porque o skyship não usa SSH:

| Criado pelo skyship | Para que serve |
|---|---|
| Role do IAM `skyship-<nome>` | A identidade **da instância**, não a sua. Leva só duas políticas gerenciadas da AWS: `AmazonSSMManagedInstanceCore`, para o agente SSM se registrar e receber o comando de deploy (é por isso que não há SSH, par de chaves nem porta 22), e `AmazonEC2ContainerRegistryReadOnly`, para a instância baixar a imagem do ECR (só leitura: ela não consegue enviar nem apagar nada). |
| Instance profile `skyship-<nome>` | A EC2 não aceita uma role diretamente; o profile é o invólucro que liga a role à instância. Tem o mesmo nome e guarda só essa role. |
| Security group `skyship-<nome>` | O firewall da instância: só a porta 80, aberta para a internet. |
| Repositório ECR `skyship/<nome>` | Onde a imagem fica guardada para a instância baixá-la. |

O seu usuário precisa destas permissões para fazer isso:

| Ação | O que o skyship faz com ela |
|---|---|
| `ecr:GetAuthorizationToken` | Fazer o login do Docker no ECR. |
| `ecr:DescribeRepositories`, `ecr:CreateRepository`, `ecr:DeleteRepository` | Ver se o repositório `skyship/<nome>` existe, criá-lo se não existir e apagá-lo (com as imagens) quando o app é removido. |
| `ecr:PutLifecyclePolicy` | Definir a política de ciclo de vida do repositório: manter só as 3 imagens mais recentes e deixar o ECR apagar as mais antigas. Opcional: sem ela o deploy funciona, mas as imagens antigas se acumulam. |
| `ecr:BatchCheckLayerAvailability`, `ecr:InitiateLayerUpload`, `ecr:UploadLayerPart`, `ecr:CompleteLayerUpload`, `ecr:PutImage` | Os passos do `docker push`: quais camadas o ECR já tem, o envio de cada camada e o registro da imagem. |
| `ec2:DescribeInstances` | Achar a instância pela tag, esperar ela ficar `running`, ler o endereço público e montar a lista de apps. |
| `ec2:DescribeInstanceStatus` | Ler os status checks (`initializing`, `ok`) mostrados no log e na lista de apps. Opcional. |
| `ec2:DescribeSecurityGroups`, `ec2:CreateSecurityGroup`, `ec2:AuthorizeSecurityGroupIngress`, `ec2:DeleteSecurityGroup` | Reaproveitar ou criar o security group, abrir a porta 80 e apagar o grupo quando o app é removido. |
| `ec2:RunInstances` | Lançar a instância na primeira vez. |
| `ec2:CreateTags` | Marcar a instância com `skyship:name` e `Name` no lançamento (só no lançamento). |
| `ec2:StartInstances` | Acordar uma instância parada quando o app é publicado de novo. |
| `ec2:TerminateInstances` | Remover o app. |
| `iam:GetRole`, `iam:GetInstanceProfile`, `iam:ListAttachedRolePolicies` | Ver se a role e o profile já existem (para reaproveitá-los) e conferir que a role é a que o skyship criou antes de apagá-la. |
| `iam:CreateRole`, `iam:CreateInstanceProfile`, `iam:AddRoleToInstanceProfile` | Criá-los na primeira vez. |
| `iam:AttachRolePolicy`, `iam:DetachRolePolicy` | Anexar as duas políticas gerenciadas à role e desanexá-las quando o app é removido (e só essas duas, veja acima). |
| `iam:RemoveRoleFromInstanceProfile`, `iam:DeleteInstanceProfile`, `iam:DeleteRole` | Apagar o instance profile e a role quando o app é removido. |
| `iam:PassRole` | Entregar a role à EC2 ao lançar a instância. Sem ela o `RunInstances` é recusado; está limitada a roles `skyship-*` e ao serviço EC2. |
| `ssm:GetParameter` | Ler o id da imagem atual do Amazon Linux no parâmetro público. |
| `ssm:DescribeInstanceInformation` | Esperar o agente SSM da instância ficar online. |
| `ssm:SendCommand`, `ssm:GetCommandInvocation` | Executar o script de deploy na instância (login no ECR, baixar a imagem, reiniciar o container) e ler se deu certo. |

### O que é removido quando você remove um app

Remover um app da EC2 encerra a instância e depois apaga o que o skyship criou para ele: o security
group, o repositório do ECR `skyship/<nome>` (com as imagens) e a role e o instance profile do IAM
`skyship-<nome>`. Nada que ele não criou é tocado:

- o security group só é apagado se a descrição for a que o skyship grava;
- a role só é apagada se a política de confiança for exatamente a do skyship, se não tiver outra
  política além das duas gerenciadas e se não houver instância do mesmo app em outra região (a role
  do IAM é global, vale para todas as regiões);
- o que não puder ser apagado é mantido, e o log diz por quê.

A limpeza também roda quando não sobrou instância (para limpar o resto de uma remoção anterior):
remova o app de novo, na pasta dele, com `skyship destroy`.

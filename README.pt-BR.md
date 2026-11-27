# skyship

**Coloque seu app Docker no ar na nuvem, sem precisar entender de nuvem.**

🇬🇧 [Read in English](README.md)

O skyship pega uma pasta com um `Dockerfile`, monta a imagem no seu computador, envia para a nuvem
e devolve um endereço público. Foi feito para quem não está acostumado com serviços de nuvem: ele
explica cada passo e remove tudo quando você terminar, para a cobrança parar.

## Como funciona

1. **Conecte a sua conta de nuvem.** O skyship diz exatamente quais permissões dar a ele.
2. **Escolha o seu app.** Uma pasta de projeto com um `Dockerfile` ou, no app de desktop, um arquivo
   compactado (`.zip`, `.tar`, `.tar.gz`, ou `.rar` se o `bsdtar` estiver instalado) com um dentro, ou
   uma imagem que já existe no seu Docker.
3. **Publique.** O skyship monta a imagem, envia e coloca o app para rodar.
4. **Abra o endereço** que ele devolve. Para atualizar o app, publique de novo com o mesmo nome.
   Quando não precisar mais, remova (`skyship destroy`, ou o botão Remover no app de desktop).

Ele publica na **AWS** (Amazon Web Services), de dois jeitos:

| Destino | Bom para |
|---|---|
| **Lightsail Containers** (padrão) | Sites e apps pequenos. Simples, preço fixo. |
| **EC2** | Quando você precisa de mais controle da máquina. |

## Começando

Você precisa de: [Docker](https://docs.docker.com/get-docker/) rodando, uma conta na AWS e o
[Rust](https://rustup.rs/) para compilar. Não há versões prontas para baixar. Foi testado no
Linux e no Windows 11 (veja abaixo).

```bash
git clone <este repositório>
cd skyship
cargo build --release          # o programa fica em target/release/skyship

cd /caminho/do/seu/projeto     # a pasta com o seu Dockerfile
skyship policy                 # mostra as permissões para dar ao skyship (veja abaixo)
skyship login                  # pede suas chaves de acesso, confere e salva
skyship init --name meuapp --target lightsail --port 80
skyship deploy
skyship status
skyship destroy --yes          # remove o app e para a cobrança
```

O `--port` é a porta em que o seu app escuta dentro do container. O app precisa responder com um
código de sucesso em `/`, porque é assim que o Lightsail confere se ele está vivo.

Também existe um app de desktop, em português, inglês e espanhol, com tema claro e escuro. Ele lista
os apps da sua conta, publica novos, atualiza os existentes e os remove. Pode sugerir o que publicar
a partir do seu Docker local e, se você ligar, verificar a imagem com o [Trivy](https://trivy.dev)
(rodando como um container do Docker) em busca de segredos e vulnerabilidades graves antes de
publicar. O que ele pode fazer no seu computador é escolhido nas Configurações.

### Instalando os programas

Não há nada para instalar além dos próprios programas: compile-os e execute-os de onde estão, ou
copie para onde quiser.

- **Linha de comando:** `cargo build --release` deixa o `skyship` (`skyship.exe`) em `target/release/`.
  `cargo install --path crates/cli` o coloca na pasta `bin` do Cargo.
- **App de desktop:** não faz parte do build padrão. Antes de compilar, instale o que ele precisa: no
  Linux as bibliotecas de desenvolvimento do WebKitGTK 4.1 (`webkit2gtk-4.1` no Arch,
  `libwebkit2gtk-4.1-dev` no Debian e no Ubuntu; veja os
  [pré-requisitos do Tauri](https://v2.tauri.app/start/prerequisites/)), no Windows as Visual Studio
  Build Tools. Depois `cargo build --release -p skyship-gui` e execute `target/release/skyship-gui`
  (mais detalhes em [docs/development.md](docs/development.md), em inglês).
- Para remover tudo, veja [Desinstalar](#desinstalar).

### Dando acesso à sua conta

Por favor, **não** use o seu login principal da AWS. Crie um usuário separado, que só pode fazer o
que o skyship precisa. O passo a passo está em [docs/permissions.pt-BR.md](docs/permissions.pt-BR.md).

## O que funciona hoje

| | Situação |
|---|---|
| `init`, `policy`, `regions` | Funcionam. |
| `doctor` | Funciona. Usado com uma conta real. |
| `login` | Escrito e a parte que grava o arquivo está testada, mas ainda não foi usado de forma interativa numa conta real. |
| `status` | Funciona no Lightsail e na EC2. |
| `deploy` no Lightsail | Funciona, inclusive atualizar um app que já existe. Testado pelo app de desktop numa conta real (contas novas podem ter limite zero de container services; talvez seja preciso pedir aumento à AWS). |
| `deploy` na EC2 | Funciona, inclusive atualizar um app. Testado pelo app de desktop numa conta real. O endereço é `http://` puro: o destino EC2 não tem HTTPS. |
| App de desktop | Funciona numa conta real da AWS: publicar, atualizar e remover apps no Lightsail e na EC2, a lista de apps e a tag do skyship, a verificação com o Trivy e publicar a partir de uma pasta de projeto, de uma imagem local ou de um arquivo compactado. |

## Desinstalar

1. **Remova os seus apps primeiro.** Desinstalar o skyship **não** remove o que ele publicou: um app
   continua rodando (e cobrando) até você removê-lo. Use **Remover** no app de desktop (Apps), ou
   `skyship destroy --yes` na pasta do app. Ao remover um app, o skyship também apaga o que criou para
   ele (na EC2: o security group, o repositório do ECR com as imagens, e a role e o profile do IAM).
2. **Remova o programa.** Ele foi compilado do código-fonte, então apague a pasta que você clonou (os
   programas ficam na pasta `target/`). Se você rodou `cargo install --path crates/cli`, rode
   `cargo uninstall skyship`.
3. **Remova o que o skyship salvou no seu computador.** Ele não salva nada em outro lugar.

   | O quê | Onde | Observações |
   |---|---|---|
   | Chaves de acesso da AWS (o perfil `skyship`, se você usou `skyship login` ou o app) | `~/.aws/credentials` (Windows: `%USERPROFILE%\.aws\credentials`) | Apague o bloco `[skyship]`. **Apague também a chave de acesso na AWS** (IAM → Users → seu usuário → Security credentials): remover o arquivo não a cancela. |
   | As configurações do app de desktop, os termos aceitos e a configuração dos apps publicados a partir de uma imagem local | Linux: `~/.config/dev.skyship.gui` e `~/.local/share/dev.skyship.gui`. Windows: `%APPDATA%\dev.skyship.gui` e `%LOCALAPPDATA%\dev.skyship.gui` | Apague as pastas. |
   | Arquivos compactados que você abriu no app | `skyship-archive-*` na pasta temporária | Removidos na próxima abertura; pode apagar. |
   | Restos no Docker | As imagens locais `skyship-build/*` (uma por deploy no Lightsail) e, se você usou a verificação com o Trivy, a imagem `aquasec/trivy:0.58.0` e o volume `skyship-trivy-cache` | `docker image rm $(docker image ls 'skyship-build/*' -q)` e `docker volume rm skyship-trivy-cache` |

## Perguntas

**Preciso da ferramenta de linha de comando da AWS?** Não.

**É seguro dar minhas chaves?** Elas são conferidas na AWS antes de serem usadas, e **você escolhe** o
que acontece depois: salvá-las no computador, em `~/.aws/credentials` (permissões restritas no Linux e
no macOS, as da sua pasta de usuário no Windows; qualquer programa que rode como você lê esse arquivo),
ou usá-las só agora, na memória do app, e elas somem quando você o fecha (só no app de desktop; nada é
gravado no computador). O app não apaga as chaves salvas depois: para removê-las, apague o perfil `[skyship]` desse arquivo e a chave de acesso na AWS (IAM → Users → Security credentials). Use o usuário dedicado de
[docs/permissions.pt-BR.md](docs/permissions.pt-BR.md), nunca a sua conta principal. Mais detalhes em
[docs/security.md](docs/security.md).

**Como atualizo um app?** Publique de novo com o mesmo nome, destino e região. O app de desktop
passa a mostrar "Atualizar <nome>". O Lightsail troca a versão; a EC2 troca o container, com alguns
segundos de indisponibilidade. O tamanho só é aplicado quando o app é criado.

**Por que o "Remover" está desabilitado para um app da lista?** A lista mostra todo container
service do Lightsail da região, mas só remove os que têm a tag `skyship:name`, que o skyship coloca
no que ele cria. Para os outros, marque a opção nas Configurações para permitir.

**Recebi "maximum limit of Lightsail Container Services".** A sua conta não tem espaço para mais um
container service. Apague um que você não usa, ou peça à AWS para aumentar o limite (Service Quotas
ou um chamado de suporte). É um limite da conta, não do skyship.

**Vou ser cobrado depois que testar?** Sim, enquanto o app existir. O `skyship destroy --yes`
(ou Remover no app de desktop) remove o serviço do Lightsail. Na EC2 ele encerra a instância e apaga também o que o skyship criou
para ela (o security group, o repositório do ECR com as imagens, e a role e o profile do IAM).

**Funciona no Windows ou no macOS?** No Windows, sim: foi compilado e testado no Windows 11 com o
Docker Desktop (backend WSL 2, modo Linux containers), e um site foi publicado no Lightsail pelo app
de desktop. Para compilar é preciso o Rust com a toolchain MSVC e o Visual Studio Build Tools (veja
[docs/development.md](docs/development.md#windows)). O macOS ainda não foi testado.

## Termos de uso

Usar o skyship significa aceitar os [termos de uso](TERMS.pt-BR.md): você é responsável pelos
custos do que ele cria na sua conta, e ele é fornecido sem garantia. O app de desktop pede o seu
aceite na primeira vez que abre.

## Sem relação com a AWS

O skyship é um projeto open source independente. Ele **não tem relação, patrocínio nem aprovação**
da Amazon Web Services. "AWS", "Amazon Lightsail", "Amazon EC2" e outros nomes são marcas de seus
donos e aparecem aqui só para dizer com quais serviços a ferramenta conversa.

## Para quem quer contribuir

A documentação técnica está em inglês:

- [docs/architecture.md](docs/architecture.md): como o código é organizado e como cada destino funciona
- [docs/security.md](docs/security.md): contra o que a ferramenta protege e o que ainda está aberto
- [docs/permissions.pt-BR.md](docs/permissions.pt-BR.md): as permissões exatas e como criar o usuário (também em inglês)
- [docs/development.md](docs/development.md): compilar, testar e a lista de lacunas conhecidas

## Licença

Licenciado sob a [Apache License 2.0](LICENSE-APACHE) ou a [licença MIT](LICENSE-MIT), à sua
escolha. As duas permitem usar, modificar e compartilhar o skyship, inclusive comercialmente, desde
que o aviso seja mantido; nenhuma delas dá qualquer garantia.

Salvo declaração em contrário, qualquer contribuição enviada para inclusão no skyship, nos termos
da Apache-2.0, é licenciada como acima, sem termos ou condições adicionais.

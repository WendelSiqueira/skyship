# Termos de uso

🇬🇧 [Read in English](TERMS.md)

Estes são os termos mostrados no app de desktop do skyship, que pede o seu aceite na primeira vez
que você o abre (e de novo sempre que eles mudarem). Versão `2026-11-26-5`.

## O que é o skyship

O skyship é um projeto open source independente. Ele não tem relação, patrocínio nem aprovação da
Amazon Web Services (AWS) nem do Docker. Esses nomes são marcas de seus donos.

## O que ele faz na sua conta

O skyship roda no seu computador. Ele monta o seu projeto com o Docker, envia para a sua conta de
nuvem e cria recursos lá (por exemplo, um serviço do Lightsail, uma instância EC2, um repositório
ECR, funções do IAM e regras de rede), usando as credenciais que você fornecer. Se você permitir,
ele também lista as imagens e os containers do Docker deste computador (nomes, tamanhos e portas)
para sugerir o que publicar, pode abrir um arquivo compactado que você escolher, numa pasta temporária e, se você ligar, roda o
Trivy (como um container do Docker, com acesso ao Docker deste computador) na imagem escolhida ou
construída antes de publicá-la.

## Custos

Os recursos criados na sua conta são cobrados de você pelo provedor de nuvem enquanto existirem. O
skyship não mostra preços. É
sua responsabilidade conferir os preços, configurar alertas de orçamento e remover o que não usa
mais. Remover um app no skyship pode não remover tudo o que foi criado.

## Suas credenciais

As chaves que você digita são validadas na AWS e salvas no arquivo de credenciais da AWS deste
computador (`~/.aws/credentials`). Se você permitir, o skyship lê os nomes dos perfis dos arquivos
de configuração da AWS deste computador (`~/.aws/credentials` e `~/.aws/config`) para você escolher
um, e usa o perfil escolhido para agir na sua conta. Use um usuário dedicado, só com as permissões
que o skyship pede. Nunca use a conta root.

## Sem garantia

O skyship é fornecido "como está", sem garantia de nenhum tipo. É uma versão inicial: um deploy pode
falhar, deixar recursos para trás ou se comportar de formas que não foram testadas.

## Limitação de responsabilidade

Na medida permitida pela lei, os autores e colaboradores não respondem por perdas ou danos
decorrentes do uso do skyship, incluindo cobranças de nuvem, perda de dados, indisponibilidade de
serviços, incidentes de segurança ou configuração incorreta da sua conta. Você usa o skyship por sua
conta e risco.

## Termos de terceiros

O uso da AWS, do Docker e de outros serviços segue os termos deles, e você é responsável por
cumpri-los.

## Mudanças

Estes termos podem mudar. Quando mudarem, o app pede que você aceite de novo.

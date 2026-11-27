# deskstats

[![build](https://github.com/danijar2000/deskstats/actions/workflows/build.yml/badge.svg)](https://github.com/danijar2000/deskstats/actions/workflows/build.yml)

[English](../README.md) · [Русский](README.ru.md) · [Español](README.es.md) · [Deutsch](README.de.md) · [Français](README.fr.md) · **Português** · [中文](README.zh.md) · [日本語](README.ja.md)

Transforme um celular Android antigo em um painel de status para o seu Mac: relógio, clima,
notificações do Mac e do iPhone, bateria do mouse e do teclado, e carga de CPU, GPU, memória
e rede com gráficos ao vivo.

- **No Mac** roda um app minúsculo em segundo plano: cerca de 4 MB de memória e meio por cento
  de um núcleo enquanto o telefone está conectado, e nada quando não está.
- **O telefone** se conecta pelo cabo USB ou por Wi‑Fi e alterna entre os dois sozinho.
  Ele dorme e acorda junto com o Mac.
- **Tudo é configurado no telefone**: idioma, tema, orientação, quais blocos mostrar,
  com que frequência atualizar, acesso por Wi‑Fi.

## Visão geral

No computador: um comando instala, duas permissões e o painel sob o ícone da barra de menus. No telefone: as páginas do app.

[![Visão geral](tour.gif)](tour.mp4)

[Ver em MP4](tour.mp4)

**O app do telefone**

[![O app do telefone](phone.gif)](phone.mp4)

[Ver em MP4](phone.mp4)

## Requisitos

- **Um computador**: um Mac com macOS 11 ou mais recente (Apple silicon ou Intel), Windows 10 ou mais recente, ou Linux.
- **Um telefone Android** com Android 2.3 ou mais recente. O app é um único arquivo: abra [deskstats.apk](https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk) no telefone, permita que o navegador instale apps quando o telefone pedir e instale. Depois ele mesmo procura uma nova versão no GitHub e oferece a atualização.
- **Opcional, um cabo USB**: assim o telefone funciona também sem Wi‑Fi e o computador faz o pareamento sozinho. É preciso adb no computador (`brew install android-platform-tools` no Mac), a depuração USB no telefone (Configurações → Sobre o telefone, toque sete vezes em "Número da versão", depois Configurações → Opções do desenvolvedor → Depuração USB) e Android 5.0 ou mais recente.

## Instalação

Conecte o telefone com um cabo e execute:

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
```

O script baixa a última [versão](https://github.com/danijar2000/deskstats/releases), instala o
app do Mac em `~/Applications/Deskstats.app` para que ele abra no login e instala o app no
telefone. Alguns segundos depois, o Mac abre o painel no telefone e faz o pareamento com ele.

Sem telefone no cabo? Adicione `-s -- --no-phone` depois de `bash` e instale
[deskstats.apk](https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk)
você mesmo.

**Sem cabo ou sem adb?** Então o script mostra o link do app: abra-o no telefone, instale o app e abra-o. Na mesma Wi‑Fi ele encontra o computador sozinho; toque nele e permita o telefone no computador. O adb só é necessário para o cabo USB.

### Permissões no Mac

Abra Ajustes do Sistema → Privacidade e Segurança e permita o **Deskstats** em:

- **Acesso Total ao Disco** — para as notificações dos apps do Mac.
- **Acessibilidade** — para as notificações encaminhadas do iPhone. O macOS as mantém apenas na
  tela, então o Deskstats as lê enquanto um banner está sendo exibido ou a Central de
  Notificações está aberta.

Todo o resto funciona sem elas. O Deskstats percebe as permissões sozinho em até meio minuto.
Depois de uma atualização, o macOS as pede de novo; se a chave já estiver ativada mas nada
chegar, remova o Deskstats da lista (−) e adicione `~/Applications/Deskstats.app` novamente.

### Linux e Windows

O servidor também funciona no Linux (x86-64 e ARM64, qualquer distribuição com systemd) e no Windows 10 ou posterior (64 bits). O app do telefone é o mesmo.

Linux, num terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
```

Windows, no PowerShell:

```powershell
irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1 | iex
```

- No Linux o servidor fica em `~/.local/bin/deskstats` e inicia no login como serviço de usuário do systemd; no Windows fica em `%LOCALAPPDATA%\deskstats`, inicia no login e grava o registro em `%APPDATA%\deskstats\log.txt`.
- Para o cabo USB instale o adb: `sudo apt install android-tools-adb` (ou o pacote da sua distribuição) ou `winget install Google.PlatformTools`.
- Para a Wi‑Fi libere TCP 8787 e UDP 8788 no firewall. Para deixar um telefone entrar pela Wi‑Fi, o Linux precisa do `zenity` ou do `kdialog` para mostrar a pergunta.
- Ainda não disponível nesses sistemas: notificações, a bateria do mouse e do teclado no Windows, a carga da GPU no Windows e no Linux fora AMD, e o telefone apagar junto com a tela.

## Uso

- **Os cartões de carga** (CPU, GPU, RAM, Rede) abrem os aplicativos do Mac ordenados por essa
  carga. A CPU é contada por núcleo, como no Monitor de Atividade: um app ocupando três núcleos
  mostra 300%.
- **O relógio e o clima** abrem a lista de cidades: adicione várias, escolha uma, remova com ×.
  O relógio, a data, o horário das notificações e o clima seguem a cidade escolhida.
- **As notificações** abrem a lista completa.
- **A engrenagem** abre os ajustes: idioma (8 idiomas), tema (escuro, claro, neon),
  orientação (paisagem, retrato, auto), os blocos da tela principal, conexão por Wi‑Fi,
  notificações do Mac, intervalo de atualização e atualização do clima.
- **Vários Macs**: os Macs com o deskstats na mesma Wi‑Fi aparecem nos ajustes, em Macs, como os
  dispositivos no LocalSend. Toque num: o telefone e o Mac mostram o mesmo código de seis dígitos e o
  Mac pergunta se deixa o telefone entrar. Ligar o telefone a um Mac por cabo uma vez também o
  adiciona. Deslize o ecrã principal para a esquerda ou para a direita para alternar; os pontos no topo
  mostram qual Mac está à vista. × esquece um Mac.
- **Voltar** fecha uma página; uma página sem uso fecha sozinha depois de alguns minutos.

Para evitar marcas na tela, a imagem se desloca alguns pixels a cada dois minutos e a tela
escurece após cinco minutos sem o Mac. O painel inicia quando o telefone reinicia (até o
Android 9; em telefones mais novos, o Mac o abre via USB).

## Atualizar e remover

Para atualizar, execute o comando de instalação de novo. Se o telefone recusar o novo app por
causa de uma assinatura diferente, desinstale o antigo (os ajustes dele são perdidos) e execute
o comando novamente:

```bash
adb uninstall app.deskstats
```

Remover o app do Mac:

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --uninstall
```

A chave de pareamento fica em `~/Library/Application Support/deskstats` (apague a pasta para
esquecer o telefone); as entradas em Privacidade e Segurança só podem ser removidas lá, à mão.

No Linux o mesmo comando remove o servidor; no Windows, no PowerShell:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1))) -Uninstall
```

A chave de emparelhamento fica em `~/.config/deskstats` no Linux e em `%APPDATA%\deskstats` no Windows.

## Solução de problemas

| O que você vê | O que fazer |
|---|---|
| "Mac · sem conexão" | Verifique o cabo e `adb devices` (o telefone deve aparecer como `device`). Por Wi‑Fi: ative a conexão por Wi‑Fi nos ajustes do painel enquanto estiver no cabo e mantenha os dois na mesma rede. |
| "falta o token: conecte o telefone por USB" | Conecte o cabo: o Mac faz o pareamento com o telefone sozinho. |
| "o servidor recusou (código N)" | O app do telefone e o app do Mac são de versões diferentes: atualize os dois. |
| Sem notificações | Conceda as permissões acima. |
| Sem clima | Escolha uma cidade: toque no clima. |

## Segurança

O telefone e o Mac criptografam tudo o que trocam com uma chave que o telefone recebe pelo cabo ou
pela Wi‑Fi quando o utilizador do Mac o permite: então combinam um segredo por ECDH (P-256), ambos
mostram um código de seis dígitos tirado dele e a chave viaja cifrada com esse segredo. Solicitações
sem a chave são recusadas antes de qualquer coisa ser executada. Cada Mac anuncia na rede local o seu
nome, modelo e endereço para que os telefones o possam listar; nada mais fica aberto. Nada sai do Mac,
exceto as coordenadas da cidade escolhida, enviadas ao open-meteo.com para obter o clima.

## Compilar a partir do código-fonte

```bash
(cd server && cargo test && ./install.sh)
(cd android && ./gradlew :app:assembleRelease)
```

O servidor precisa de Rust; o app precisa do JDK 17+ e do Android SDK com a plataforma 34.
O [GitHub Actions](../.github/workflows/build.yml) compila os dois a cada push e publica uma
versão para cada tag `v*`.

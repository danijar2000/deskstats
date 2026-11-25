# deskstats

[![build](https://github.com/danijar2000/deskstats/actions/workflows/build.yml/badge.svg)](https://github.com/danijar2000/deskstats/actions/workflows/build.yml)

[English](../README.md) · [Русский](README.ru.md) · [Español](README.es.md) · [Deutsch](README.de.md) · [Français](README.fr.md) · [Português](README.pt.md) · [中文](README.zh.md) · **日本語**

古い Android スマートフォンを Mac のステータスパネルに変えます。時計、天気、Mac と iPhone の通知、
マウスとキーボードのバッテリー、そして CPU・GPU・メモリ・ネットワークの負荷をライブグラフで表示します。

![Mac とその横に置いたパネル用スマートフォン](screenshots/hero.png)

- **Mac 側**は小さなバックグラウンドアプリです。スマートフォン接続中のメモリは約 4 MB、CPU は 1 コアの
  0.5% ほどで、接続していないときは何も消費しません。
- **スマートフォン**は USB ケーブルまたは Wi‑Fi で接続し、両者を自動で切り替えます。
  Mac と一緒にスリープし、一緒に復帰します。
- **設定はすべてスマートフォン上で**：言語、テーマ、画面の向き、表示するブロック、更新頻度、Wi‑Fi アクセス。

## スクリーンショット

| | |
|---|---|
| ![ダークテーマ](screenshots/main-dark.png) | ![ネオンテーマ](screenshots/main-neon.png) |
| **ダーク**テーマ（デフォルト） | **ネオン**テーマ |
| ![ライトテーマ](screenshots/main-light.png) | ![縦向き](screenshots/main-portrait.png) |
| **ライト**テーマ | **縦向き** |
| ![アプリ](screenshots/apps.png) | ![通知](screenshots/alerts.png) |
| 負荷カードをタップ：その負荷順に並んだ Mac のアプリ | 通知をタップ：本文付きの全リスト |
| ![都市](screenshots/places.png) | ![設定](screenshots/settings.png) |
| 時計か天気をタップ：都市を選択 | 歯車：設定 |

## 必要なもの

- **Mac**：macOS 11 以降（Apple シリコンまたは Intel）。
- **Android スマートフォン**：Android 2.3 以降。USB 接続には Android 5.0 が必要です。古い機種は
  Wi‑Fi のみで使えます。
- USB 接続用に Mac に **adb**：`brew install android-platform-tools`。
- スマートフォンで **USB デバッグ**を有効に：設定 → デバイス情報で「ビルド番号」を 7 回タップし、
  設定 → 開発者向けオプション → USB デバッグ をオンにします。確認が表示されたら、この Mac からのデバッグを許可してください。

## インストール

スマートフォンをケーブルで接続して、次を実行します。

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
```

最新の[リリース](https://github.com/danijar2000/deskstats/releases)をダウンロードし、Mac アプリを
`~/Applications/Deskstats.app` にインストールしてログイン時に起動するよう設定し、スマートフォンにもアプリを
インストールします。数秒後、Mac がスマートフォン上でパネルを開き、ペアリングします。

スマートフォンがケーブルにつながっていない場合は、`bash` の後に `-s -- --no-phone` を付け、
[deskstats.apk](https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk)
を自分でインストールしてください。

**ケーブルや adb がない場合：** スクリプトがアプリへのリンクの QR コードを表示します。スマートフォンで読み取り、アプリをインストールして開いてください。同じ Wi‑Fi にあればアプリがコンピュータを自動で見つけます。タップして、コンピュータ側でスマートフォンを許可してください。adb が必要なのは USB ケーブルを使うときだけです。

### Mac のアクセス許可

システム設定 → プライバシーとセキュリティ を開き、**Deskstats** に次を許可します。

- **フルディスクアクセス** — Mac アプリの通知のため。
- **アクセシビリティ** — iPhone から転送された通知のため。macOS はこれらを画面上にしか保持しないため、
  Deskstats はバナーの表示中または通知センターが開いている間に読み取ります。

それ以外の機能は許可なしでも動作します。Deskstats は 30 秒以内に許可を自動で検知します。アップデート後は
macOS が再び許可を求めます。スイッチがすでにオンなのに何も届かない場合は、リストから Deskstats を削除（−）し、
`~/Applications/Deskstats.app` を追加し直してください。

### Linux と Windows

サーバーは Linux（x86-64 と ARM64、systemd のあるディストリビューション）と Windows 10 以降（64 ビット）でも動きます。スマートフォンのアプリは同じものです。

Linux（ターミナルで）：

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
```

Windows（PowerShell で）：

```powershell
irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1 | iex
```

- Linux ではサーバーは `~/.local/bin/deskstats` に置かれ、ログイン時に systemd のユーザーサービスとして起動します。Windows では `%LOCALAPPDATA%\deskstats` に置かれ、サインイン時に起動し、ログは `%APPDATA%\deskstats\log.txt` に書かれます。
- USB ケーブルで使うには adb をインストールします：`sudo apt install android-tools-adb`（またはお使いのディストリビューションのパッケージ）、または `winget install Google.PlatformTools`。
- Wi‑Fi で使うには、ファイアウォールで TCP 8787 と UDP 8788 を許可します。Wi‑Fi でスマートフォンを許可するには、Linux では確認ウインドウを出すために `zenity` か `kdialog` が必要です。
- これらのシステムでは未対応：通知、Windows でのマウスとキーボードのバッテリー、Windows と AMD 以外の Linux での GPU 負荷、ディスプレイに合わせたスマートフォンの消灯。

## 使い方

- **負荷カード**（CPU、GPU、RAM、ネットワーク）をタップすると、その負荷順に並んだ Mac のアプリが開きます。
  CPU はアクティビティモニタと同じくコア単位で数えるため、3 コアを使うアプリは 300% と表示されます。
- **時計と天気**をタップすると都市リストが開きます。複数追加し、1 つを選び、× で削除できます。
  時計、日付、通知の時刻、天気は選んだ都市に従います。
- **通知**をタップすると全リストが開きます。
- **歯車**をタップすると設定が開きます：言語（8 言語）、テーマ（ダーク、ライト、ネオン）、
  画面の向き（横向き、縦向き、自動）、メイン画面のブロック、Wi‑Fi 接続、Mac の通知、更新間隔、天気の更新。
- **複数の Mac**：同じ Wi‑Fi で deskstats が動いている Mac は、LocalSend のデバイスのように設定の「Mac」に表示されます。
  タップすると、スマートフォンと Mac に同じ 6 桁のコードが表示され、Mac がスマートフォンを許可するか尋ねます。
  一度ケーブルで接続して追加することもできます。メイン画面を左右にスワイプすると切り替わり、上部のドットで
  表示中の Mac がわかります。× で Mac を削除します。
- **戻る**でページを閉じます。操作のないページは数分後に自動で閉じます。

焼き付き防止のため、画像は 2 分ごとに数ピクセル移動し、Mac との接続がない状態が 5 分続くと画面が暗くなります。
パネルはスマートフォンの再起動後に自動で起動します（Android 9 まで。それより新しい機種では Mac が USB 経由で開きます）。

## アップデートと削除

アップデート：インストールコマンドをもう一度実行します。署名が異なるためにスマートフォンが新しいアプリを
拒否した場合は、古いアプリを削除して（設定は失われます）、もう一度コマンドを実行してください。

```bash
adb uninstall app.deskstats
```

Mac アプリを削除するには：

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --uninstall
```

ペアリング用の鍵は `~/Library/Application Support/deskstats` に残ります（スマートフォンの登録を消すにはこの
フォルダを削除してください）。「プライバシーとセキュリティ」の項目は、そこで手動でしか削除できません。

## トラブルシューティング

| 表示 | 対処 |
|---|---|
| 「Mac · 接続なし」 | ケーブルと `adb devices` を確認してください（スマートフォンが `device` として表示される必要があります）。Wi‑Fi の場合：ケーブル接続中にパネルの設定で「Wi‑Fi 接続」をオンにし、両方を同じネットワークにつないでください。 |
| 「トークンが必要です：USB で接続してください」 | ケーブルを接続してください。Mac が自動でペアリングします。 |
| 「サーバーが拒否しました（コード N）」 | スマートフォンのアプリと Mac のアプリのバージョンが異なります。両方をアップデートしてください。 |
| 通知が出ない | 上記のアクセス許可を与えてください。 |
| 天気が出ない | 天気をタップして都市を選んでください。 |

## セキュリティ

スマートフォンと Mac は、やり取りするすべてのデータを鍵で暗号化します。スマートフォンはこの鍵をケーブル経由で、
または Mac のユーザーが許可したときに Wi‑Fi 経由で受け取ります。その際、両者は ECDH（P-256）で秘密を共有し、
そこから作られた 6 桁のコードを双方に表示し、鍵はその秘密で暗号化されて送られます。鍵のないリクエストは、
何かが実行される前に拒否されます。各 Mac はスマートフォンが一覧に表示できるよう、名前・モデル・アドレスを
ローカルネットワークに知らせます。それ以外は公開されません。天気の取得のために open-meteo.com へ送る
選択した都市の座標以外、Mac から外部に出るデータはありません。

## ソースからビルド

```bash
(cd server && cargo test && ./install.sh)
(cd android && ./gradlew :app:assembleRelease)
```

サーバーには Rust が、アプリには JDK 17 以降とプラットフォーム 34 を含む Android SDK が必要です。
[GitHub Actions](../.github/workflows/build.yml) がプッシュのたびに両方をビルドし、`v*` タグごとにリリースを公開します。

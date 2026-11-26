# deskstats

[![build](https://github.com/danijar2000/deskstats/actions/workflows/build.yml/badge.svg)](https://github.com/danijar2000/deskstats/actions/workflows/build.yml)

[English](../README.md) · [Русский](README.ru.md) · [Español](README.es.md) · **Deutsch** · [Français](README.fr.md) · [Português](README.pt.md) · [中文](README.zh.md) · [日本語](README.ja.md)

Mach aus einem alten Android-Telefon ein Statuspanel für deinen Mac: Uhr, Wetter, Mitteilungen
vom Mac und vom iPhone, Akkustand von Maus und Tastatur sowie CPU-, GPU-, Speicher- und
Netzwerklast mit Live-Diagrammen.

![Ein Mac mit dem Telefon-Panel daneben](screenshots/hero.png)

- **Auf dem Mac** läuft eine winzige Hintergrund-App: etwa 4 MB Speicher und ein halbes Prozent
  eines Kerns, solange das Telefon verbunden ist, und gar nichts, wenn nicht.
- **Das Telefon** verbindet sich per USB-Kabel oder WLAN und wechselt selbstständig zwischen
  beiden. Es schläft und wacht zusammen mit dem Mac.
- **Alles wird am Telefon eingestellt**: Sprache, Design, Ausrichtung, welche Blöcke angezeigt
  werden, wie oft aktualisiert wird, WLAN-Zugriff.

## Screenshots

| | |
|---|---|
| ![Dunkles Design](screenshots/main-dark.png) | ![Design Neon](screenshots/main-neon.png) |
| Design **Dunkel**, die Voreinstellung | Design **Neon** |
| ![Helles Design](screenshots/main-light.png) | ![Hochformat](screenshots/main-portrait.png) |
| Design **Hell** | Ausrichtung **Hoch** |
| ![Apps](screenshots/apps.png) | ![Mitteilungen](screenshots/alerts.png) |
| Tippe auf eine Lastkarte: die Apps des Macs, sortiert nach dieser Last | Tippe auf die Mitteilungen: die vollständige Liste mit Texten |
| ![Städte](screenshots/places.png) | ![Einstellungen](screenshots/settings.png) |
| Tippe auf Uhr oder Wetter: Stadt wählen | Das Zahnrad: Einstellungen |

## Voraussetzungen

- **Ein Mac** mit macOS 11 oder neuer (Apple Silicon oder Intel).
- **Ein Android-Telefon** mit Android 2.3 oder neuer. Die USB-Verbindung braucht Android 5.0;
  ältere Telefone nutzen nur WLAN.
- **adb** auf dem Mac für die USB-Verbindung: `brew install android-platform-tools`.
- **USB-Debugging** auf dem Telefon: Einstellungen → Über das Telefon, siebenmal auf
  „Build-Nummer“ tippen, dann Einstellungen → Entwickleroptionen → USB-Debugging. Erlaube das
  Debugging für diesen Mac, wenn das Telefon fragt.

## Installation

Telefon per Kabel anschließen und ausführen:

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
```

Das Skript lädt das neueste [Release](https://github.com/danijar2000/deskstats/releases), installiert
die Mac-App unter `~/Applications/Deskstats.app`, sodass sie bei der Anmeldung startet, und
installiert die App auf dem Telefon. Wenige Sekunden später öffnet der Mac das Panel auf dem
Telefon und koppelt sich mit ihm.

Kein Telefon am Kabel? Hänge `-s -- --no-phone` hinter `bash` an und installiere
[deskstats.apk](https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk)
selbst.

**Kein Kabel oder kein adb?** Dann zeigt das Skript einen QR-Code mit dem Link zur App: mit dem Telefon scannen, die App installieren und öffnen. Im selben WLAN findet sie den Computer von selbst; tippe ihn an und erlaube das Telefon auf dem Computer. adb wird nur für das USB-Kabel gebraucht.

### Berechtigungen auf dem Mac

Öffne Systemeinstellungen → Datenschutz & Sicherheit und erlaube **Deskstats**:

- **Festplattenvollzugriff** – für die Mitteilungen von Mac-Apps.
- **Bedienungshilfen** – für Mitteilungen, die vom iPhone weitergeleitet werden. macOS behält
  diese nur auf dem Bildschirm, daher liest Deskstats sie, solange ein Banner angezeigt wird oder
  die Mitteilungszentrale geöffnet ist.

Alles andere funktioniert auch ohne sie. Deskstats erkennt die Berechtigungen innerhalb einer
halben Minute von selbst. Nach einem Update fragt macOS erneut danach; ist ein Schalter bereits
an, kommt aber nichts an, entferne Deskstats aus der Liste (−) und füge
`~/Applications/Deskstats.app` erneut hinzu.

### Linux und Windows

Der Server läuft auch unter Linux (x86-64 und ARM64, jede Distribution mit systemd) und unter Windows 10 oder neuer (64 Bit). Die Telefon-App ist dieselbe.

Linux, im Terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash
```

Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1 | iex
```

- Unter Linux liegt der Server in `~/.local/bin/deskstats` und startet bei der Anmeldung als systemd-Benutzerdienst; unter Windows liegt er in `%LOCALAPPDATA%\deskstats`, startet bei der Anmeldung und schreibt sein Protokoll nach `%APPDATA%\deskstats\log.txt`.
- Für das USB-Kabel adb installieren: `sudo apt install android-tools-adb` (oder das Paket deiner Distribution) bzw. `winget install Google.PlatformTools`.
- Für WLAN TCP 8787 und UDP 8788 in der Firewall freigeben. Um ein Telefon per WLAN hereinzulassen, braucht Linux `zenity` oder `kdialog` für die Rückfrage.
- Auf diesen Systemen noch nicht vorhanden: Mitteilungen, der Akkustand von Maus und Tastatur unter Windows, die GPU-Last unter Windows und unter Linux außer bei AMD sowie das Abdunkeln des Telefons mit dem Bildschirm.

## Bedienung

- **Lastkarten** (CPU, GPU, RAM, Netz) öffnen die Apps des Macs, sortiert nach dieser Last. CPU
  wird pro Kern gezählt, wie in der Aktivitätsanzeige: Eine App, die drei Kerne auslastet,
  zeigt 300 %.
- **Uhr und Wetter** öffnen die Städteliste: mehrere hinzufügen, eine auswählen, mit ×
  entfernen. Uhr, Datum, Mitteilungszeiten und Wetter richten sich nach der gewählten Stadt.
- **Mitteilungen** öffnen die vollständige Liste.
- **Das Zahnrad** öffnet die Einstellungen: Sprache (8 Sprachen), Design (dunkel, hell, Neon),
  Ausrichtung (quer, hoch, auto), die Blöcke des Startbildschirms, WLAN-Verbindung,
  Mac-Mitteilungen, Aktualisierungsintervall und Wetteraktualisierung.
- **Mehrere Macs**: Macs mit deskstats im selben WLAN erscheinen in den Einstellungen unter Macs, wie
  Geräte in LocalSend. Tippe auf einen: Telefon und Mac zeigen denselben sechsstelligen Code, und der
  Mac fragt, ob er das Telefon hereinlassen soll. Einmal per Kabel verbunden, wird ein Mac ebenfalls
  hinzugefügt. Ein Wisch über den Hauptbildschirm nach links oder rechts wechselt den Mac; die Punkte
  oben zeigen, welcher angezeigt wird. × vergisst einen Mac.
- **Zurück** schließt eine Seite; eine unbeachtete Seite schließt sich nach ein paar Minuten
  von selbst.

Gegen Einbrennen verschiebt sich das Bild alle zwei Minuten um ein paar Pixel, und nach fünf
Minuten ohne Mac wird der Bildschirm gedimmt. Das Panel startet nach einem Neustart des Telefons
(bis Android 9; auf neueren Telefonen öffnet es der Mac über USB).

## Aktualisieren und entfernen

Aktualisieren: den Installationsbefehl erneut ausführen. Lehnt das Telefon die neue App wegen
einer anderen Signatur ab, entferne die alte (ihre Einstellungen gehen verloren) und führe den
Befehl erneut aus:

```bash
adb uninstall app.deskstats
```

Mac-App entfernen:

```bash
curl -fsSL https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.sh | bash -s -- --uninstall
```

Der Kopplungsschlüssel bleibt in `~/Library/Application Support/deskstats` (lösche den Ordner,
um das Telefon zu vergessen); die Einträge unter Datenschutz & Sicherheit lassen sich nur dort
von Hand entfernen.

Unter Linux entfernt derselbe Befehl den Server; unter Windows in PowerShell:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1))) -Uninstall
```

Der Kopplungsschlüssel bleibt unter Linux in `~/.config/deskstats` und unter Windows in `%APPDATA%\deskstats`.

## Fehlerbehebung

| Was du siehst | Was zu tun ist |
|---|---|
| „Mac · keine Verbindung“ | Prüfe das Kabel und `adb devices` (das Telefon muss als `device` aufgeführt sein). Über WLAN: Schalte die WLAN-Verbindung in den Einstellungen des Panels ein, während das Telefon am Kabel hängt, und halte beide Geräte im selben Netz. |
| „Token fehlt: Telefon per USB verbinden“ | Schließe das Kabel an: Der Mac koppelt das Telefon von selbst. |
| „Server hat abgelehnt (Code N)“ | Telefon-App und Mac-App haben unterschiedliche Versionen: Aktualisiere beide. |
| Keine Mitteilungen | Erteile die oben genannten Berechtigungen. |
| Kein Wetter | Wähle eine Stadt: Tippe auf das Wetter. |

## Sicherheit

Telefon und Mac verschlüsseln alles, was sie einander senden, mit einem Schlüssel, den das
Telefon über das Kabel erhält oder über WLAN, sobald der Benutzer des Mac es erlaubt: Dann einigen
sich Telefon und Mac per ECDH (P-256) auf ein Geheimnis, beide zeigen einen sechsstelligen Code
daraus, und der Schlüssel reist damit verschlüsselt. Anfragen ohne Schlüssel werden abgelehnt, bevor
irgendetwas ausgeführt wird. Jeder Mac meldet im lokalen Netzwerk Namen, Modell und Adresse, damit
Telefone ihn auflisten können; sonst ist nichts offen. Nichts verlässt den Mac außer den Koordinaten
der gewählten Stadt, die für das Wetter an open-meteo.com gesendet werden.

## Aus dem Quellcode bauen

```bash
(cd server && cargo test && ./install.sh)
(cd android && ./gradlew :app:assembleRelease)
```

Der Server braucht Rust; die App braucht JDK 17+ und das Android SDK mit Plattform 34.
[GitHub Actions](../.github/workflows/build.yml) baut beides bei jedem Push und veröffentlicht für
jedes `v*`-Tag ein Release.

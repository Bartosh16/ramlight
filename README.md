# RamLight

Lekki monitor RAM dla Windows 11. W zasobniku pokazuje literę **R** w kolorze stanu pamięci; kliknięcie otwiera chwilowe okno z trendem i procesami zużywającymi najwięcej RAM. Zamknięcie okna niszczy WebView2, więc w tle pozostaje tylko proces Rust. Interfejs jest po polsku.

## Pobierz

Gotowy plik exe jest w [Releases](https://github.com/Bartosh16/ramlight/releases/latest). Wrzuć go do dowolnego folderu i uruchom. Plik nie jest podpisany, więc Windows SmartScreen pokaże ostrzeżenie: „Więcej informacji” → „Uruchom mimo to”. Kto woli nie ufać binarce, buduje ze źródeł (sekcja „Budowanie”).

## Progi

Rezerwa `R = max(1 GiB, 8% całego RAM)`. Slot typowego procesu `S = 2 GiB` domyślnie; można go zmienić w ustawieniach od 0,5 do 16 GB. Poziomy liczone są z wolnej pamięci uśrednionej z trzech pomiarów co 2 sekundy:

| Kolor | Stan | Warunek |
| --- | --- | --- |
| Zielony | Czysto | wolne ≥ R + 2S |
| Żółty | Zmieści się jeden proces | wolne ≥ R + S |
| Pomarańczowy | Kolejny proces = ryzyko | wolne ≥ R |
| Czerwony | Niczego nie dokładaj | wolne < R |

Commit charge ≥90% wymusza co najmniej pomarańczowy, a ≥97% czerwony. Zmiana koloru wymaga dwóch kolejnych zgodnych pomiarów. Trend to regresja liniowa z ostatnich pięciu minut.

## Co wolno ubić

Lista grupuje procesy według nazwy i sumuje ich working set. Każda grupa ma kolor z wbudowanego katalogu (`categorize()` w `src-tauri/src/main.rs`):

| Kolor | Kategoria | Przykłady | Co się dzieje po kliknięciu „Ubij” |
| --- | --- | --- | --- |
| Zielony | bezpieczny | Chrome, Edge, Brave, Discord, Slack, Spotify, Notion, launchery gier | zwykłe potwierdzenie; po ponownym uruchomieniu wszystko wraca (sprawdź tylko, czy nic się nie pobiera) |
| Żółty | ryzyko | VS Code, Word, Excel, Notatnik, WebView2 innych aplikacji, `node.exe`, OneDrive, Docker | potwierdzenie z ostrzeżeniem o niezapisanych zmianach |
| Pomarańczowy | nie ubijaj | `ffmpeg`, `python`, DaVinci Resolve, OBS, Claude, Codex, kompilacja, pakowanie archiwum, instalator | potwierdzenie „stracisz pracę w toku” |
| Czerwony | system | `svchost`, `csrss`, `lsass`, `dwm`, `explorer`, Memory Compression | brak przycisku; Rust odrzuca ubicie także przy ręcznym wywołaniu |
| Szary | nieznany | wszystko spoza katalogu | potwierdzenie „nie wiem, co to jest” |

Przycisk **Ubij wszystkie zbędne** ubija tylko zielone. Listę liczy od nowa Rust w chwili kliknięcia, więc żółte, pomarańczowe, szare i systemowe zostają nietknięte. Katalog łatwo rozszerzyć – pull requesty z kolejnymi programami mile widziane.

Używaj ubijania z głową: proces kończony siłowo nie zapisuje danych.

Ustawienia są w `%APPDATA%\RamLight\settings.json`. Przy pierwszym uruchomieniu RamLight włącza autostart; można go przełączyć w menu zasobnika.

## Budowanie

Wymagania: Windows 11, Rust/Cargo, MSVC Build Tools, WebView2 Runtime. Projekt używa Tauri 2 z czystym HTML/CSS/JS w `ui/` i nie wymaga Node ani Vite.

```powershell
cargo install tauri-cli --version "^2" --locked
cd src-tauri
cargo tauri build
```

Testy progów i listy chronionych procesów:

```powershell
cargo test
```

Zwykły `cargo build --release` nie wystarczy: bez CLI Tauri plik exe nie osadza interfejsu z `ui/`.

Gotowy plik: `src-tauri/target/release/ramlight.exe`. Licencja MIT.

## Autor

Daniel Bartosiewicz, [DBest Content](https://dbest-content.com) – automatyzacja contentu z AI. RamLight powstał, bo przy montażu wideo i kilku sesjach Claude Code naraz 16 GB RAM-u kończyło się szybciej, niż dało się to zauważyć.

# Snipping Tool (Rust)

Aplikasi snipping ala Windows untuk Linux, ditulis dengan Rust (eframe/egui).

## Fitur

| Fitur | Keterangan |
|---|---|
| ✂ **New** (Ctrl+N) | Ambil screenshot. Mode **Rectangle** (drag area di layar yang dibekukan) atau **Full screen**. Bisa diberi delay 3/5/10 detik. |
| 🖊 **Pen** (P) | Coretan bebas, 7 warna preset + pemilih warna, ukuran 1–30 px |
| 🖍 **Highlighter** (H) | Stabilo transparan (blend *multiply*: teks hitam tetap terbaca) |
| ⌫ **Eraser** (E) | Drag di atas coretan untuk menghapus coretan tersebut |
| ✂ **Crop** (C) | Drag area yang mau disimpan → **Enter**/✔ Apply, **Esc** batal |
| ⟲ Undo / ⟳ Redo | Ctrl+Z / Ctrl+Y (atau Ctrl+Shift+Z), termasuk crop |
| 📋 Copy (Ctrl+C) | Salin ke clipboard. Setiap snip baru otomatis disalin. |
| 💾 Save (Ctrl+S) | Dialog simpan bawaan sistem, PNG atau JPEG |

Di layar seleksi: **drag** = pilih area, **Enter** = seluruh layar, **Esc** / klik kanan = batal.

## Build & jalankan

```bash
# Pasang Rust (sekali saja)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

cargo run --release                 # buka editor
cargo run --release -- --snip       # langsung snip
cargo test                          # unit test (render pen/highlighter, crop, undo, eraser, ikon)
```

Tidak butuh paket `-dev` atau `pkg-config`: semua dependency murni Rust, dan library Wayland/X11/EGL dimuat saat runtime.

### Pasang ke menu aplikasi (dengan ikon)

```bash
cargo build --release
./target/release/snipping-tool --install     # pasang
snipping-tool --uninstall                    # hapus lagi
```

`--install` menyalin binary ke `~/.local/bin`, ikon (16–512 px) ke `~/.local/share/icons/hicolor`, dan
`snipping-tool.desktop` ke `~/.local/share/applications`. Di Wayland, ikon jendela diambil dari file
`.desktop` yang cocok dengan app id `snipping-tool`, jadi langkah ini diperlukan agar ikon tampil di
dock dan Alt+Tab. Ikon digambar di kode (`src/icon.rs`), tanpa file gambar terpisah.

Bonus: kalau dibuka dari menu aplikasi, portal mengenali aplikasi ini sebagai `snipping-tool`
(bukan terminal), sehingga GNOME bisa menanyakan izin screenshot untuk aplikasi ini sendiri.

### Shortcut keyboard global (seperti Win+Shift+S)

GNOME: *Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts*, tambahkan
perintah `snipping-tool --snip` (setelah `--install`).

## Cara kerja

- **Capture** lewat `xdg-desktop-portal` (crate `ashpd`), satu-satunya cara resmi untuk screenshot di Wayland.
  Aplikasi mencoba capture diam-diam (non-interaktif) dulu. GNOME hanya mengizinkan ini kalau
  "aplikasi pemanggil" punya izin screenshot. GNOME menentukan aplikasi pemanggil dari *systemd scope*,
  jadi kalau dijalankan dari terminal VS Code, yang dicek adalah izin VS Code. Kalau ditolak, aplikasi
  memakai **UI screenshot bawaan GNOME** untuk memilih area (layar seleksi milik aplikasi dilewati).
  File sementara yang dibuat portal langsung dihapus setelah dibaca.
- Jendela editor **ditutup** sebelum capture, lalu dibuka lagi sesudahnya (`main.rs`), karena di Wayland
  aplikasi tidak bisa menyembunyikan jendelanya sendiri. Capture berjalan di jendela 1×1 transparan
  (`capture.rs`) agar event loop tetap berjalan. Di Wayland, winit baru benar-benar menghapus jendela
  lama saat event loop berjalan, jadi tanpa ini editor lama tetap tampil beku.
- Coretan disimpan sebagai **vektor** (`document.rs`), lalu digambar ke gambar dengan `tiny-skia`
  saat ditampilkan atau diekspor. Akibatnya crop dan undo tidak mengurangi kualitas.

```
src/
├── main.rs      alur: editor → capture → seleksi → editor
├── capture.rs   fase capture (jendela transparan, delay)
├── portal.rs    screenshot + dialog Save via xdg-desktop-portal
├── selector.rs  overlay fullscreen untuk memilih area
├── editor.rs    jendela utama: toolbar, kanvas, tool
├── document.rs  gambar + coretan, undo/redo, crop, render
├── session.rs   pengaturan, clipboard, simpan file
├── gfx.rs       helper gambar (texture, dim, label)
├── icon.rs      ikon aplikasi (digambar dengan tiny-skia)
└── install.rs   --install / --uninstall ke ~/.local
```

## Keterbatasan

- Butuh layanan `xdg-desktop-portal` (sudah ada di GNOME/KDE; di wlroots perlu `xdg-desktop-portal-wlr`).
- Pada multi-monitor, seleksi ditampilkan di satu monitor (screenshot gabungan diperkecil agar muat).
- Di GNOME, isi clipboard hilang ketika aplikasi ditutup (tidak ada clipboard manager bawaan).

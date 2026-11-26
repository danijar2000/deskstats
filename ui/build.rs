// The QR code is drawn by the server's own encoder (server/src/qr.rs); this tells its tests where they are built.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(deskstats_ui)");
    println!("cargo::rustc-cfg=deskstats_ui");
}

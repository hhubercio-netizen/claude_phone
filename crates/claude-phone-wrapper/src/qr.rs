use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;

pub fn render_terminal(s: &str) -> String {
    // TM-CODE.3: QrCode::new only fails on inputs longer than the QR
    // alphanumeric capacity (~4296 chars). Pairing URLs are <250 chars,
    // so this is infallible for our use.
    let code = QrCode::new(s)
        .expect("pairing URL fits in default QR version (input <250 chars, max ~4000)");
    code.render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .build()
}

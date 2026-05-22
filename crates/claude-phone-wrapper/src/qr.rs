use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;

pub fn render_terminal(s: &str) -> String {
    let code = QrCode::new(s).expect("qr encoding");
    code.render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .build()
}

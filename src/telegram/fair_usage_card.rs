use super::svg_render::fmt_bytes;
use crate::fair_usage::FairUsagePeerStatusDTO;

pub fn generate_fair_usage_card_svg(dto: &FairUsagePeerStatusDTO, peer_name: &str) -> String {
    let width = 640.0;
    let height = 300.0;
    let is_throttled = dto.throttled;
    let status_color = if is_throttled { "#dc2626" } else { "#16a34a" };
    let status_bg = if is_throttled { "#fee2e2" } else { "#dcfce7" };
    let status_text = if is_throttled { "Throttled" } else { "Normal" };

    let used_combined = dto.used_rx + dto.used_tx;
    let quota_total = dto.download_quota_bytes;
    let pct = if quota_total > 0 {
        ((used_combined as f64 / quota_total as f64) * 100.0).min(100.0)
    } else {
        0.0
    };

    let bar_width = 520.0;
    let fill_width = (bar_width * (pct / 100.0)).max(0.0);
    let bar_color = if is_throttled { "#dc2626" } else { "#2563eb" };

    let mut out = String::new();
    out.push_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n", width, height, width, height));
    out.push_str(&format!("  <rect width=\"{}\" height=\"{}\" rx=\"16\" fill=\"#ffffff\"/>\n", width, height));
    out.push_str("  <text x=\"40\" y=\"44\" font-family=\"Vazirmatn\" font-size=\"20\" font-weight=\"bold\" fill=\"#111827\">Fair Usage Policy</text>\n");
    out.push_str(&format!("  <text x=\"40\" y=\"68\" font-family=\"Vazirmatn\" font-size=\"14\" fill=\"#6b7280\">{}</text>\n", peer_name));
    out.push_str(&format!("  <rect x=\"480\" y=\"30\" width=\"100\" height=\"28\" rx=\"14\" fill=\"{}\"/>\n", status_bg));
    out.push_str(&format!("  <text x=\"530\" y=\"49\" font-family=\"Vazirmatn\" font-size=\"13\" font-weight=\"bold\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n", status_color, status_text));

    let quota_str = if quota_total > 0 { fmt_bytes(quota_total) } else { "Unlimited".to_string() };
    out.push_str(&format!("  <text x=\"40\" y=\"125\" font-family=\"Vazirmatn\" font-size=\"13\" fill=\"#374151\">Usage: {} / {} ({:.1}%)</text>\n", fmt_bytes(used_combined), quota_str, pct));
    out.push_str(&format!("  <rect x=\"40\" y=\"140\" width=\"{}\" height=\"14\" rx=\"7\" fill=\"#f3f4f6\"/>\n", bar_width));
    out.push_str(&format!("  <rect x=\"40\" y=\"140\" width=\"{}\" height=\"14\" rx=\"7\" fill=\"{}\"/>\n", fill_width, bar_color));

    out.push_str("  <rect x=\"40\" y=\"185\" width=\"250\" height=\"70\" rx=\"12\" fill=\"#f9fafb\"/>\n");
    out.push_str("  <text x=\"60\" y=\"212\" font-family=\"Vazirmatn\" font-size=\"12\" fill=\"#6b7280\">Speed Limit</text>\n");
    out.push_str(&format!("  <text x=\"60\" y=\"238\" font-family=\"Vazirmatn\" font-size=\"16\" font-weight=\"bold\" fill=\"#111827\">&darr; {} Kbps / &uarr; {} Kbps</text>\n", dto.throttle_download_kbps, dto.throttle_upload_kbps));

    out.push_str("  <rect x=\"310\" y=\"185\" width=\"250\" height=\"70\" rx=\"12\" fill=\"#f9fafb\"/>\n");
    out.push_str("  <text x=\"330\" y=\"212\" font-family=\"Vazirmatn\" font-size=\"12\" fill=\"#6b7280\">Reset Cycle</text>\n");
    out.push_str(&format!("  <text x=\"330\" y=\"238\" font-family=\"Vazirmatn\" font-size=\"16\" font-weight=\"bold\" fill=\"#111827\">{}</text>\n", dto.scope_label));
    out.push_str("</svg>\n");

    out
}

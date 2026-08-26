use super::svg_render::fmt_bytes;

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&apos;")
}

pub fn generate_usage_chart_svg(
    title: &str,
    peer_name: &str,
    rx_total: i64,
    tx_total: i64,
    points: &[(String, i64, i64)],
) -> String {
    let width = 640.0;
    let height = 360.0;
    let margin_left = 60.0;
    let margin_right = 30.0;
    let margin_top = 80.0;
    let margin_bottom = 50.0;

    let chart_w = width - margin_left - margin_right;
    let chart_h = height - margin_top - margin_bottom;

    let max_val = points
        .iter()
        .map(|(_, rx, tx)| (*rx).max(*tx))
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let total_combined = rx_total + tx_total;

    let mut rx_path_points = Vec::new();
    let mut tx_path_points = Vec::new();

    let n = points.len();
    for (i, &(_, rx, tx)) in points.iter().enumerate() {
        let x = if n <= 1 {
            margin_left + chart_w / 2.0
        } else {
            margin_left + (i as f64 / (n - 1) as f64) * chart_w
        };
        let y_rx = margin_top + chart_h - (rx as f64 / max_val) * chart_h;
        let y_tx = margin_top + chart_h - (tx as f64 / max_val) * chart_h;

        rx_path_points.push((x, y_rx));
        tx_path_points.push((x, y_tx));
    }

    let mut rx_d = String::new();
    let mut tx_d = String::new();

    if n == 1 {
        let (_x, y_rx) = rx_path_points[0];
        let (_, y_tx) = tx_path_points[0];
        rx_d = format!("M {:.1},{:.1} L {:.1},{:.1}", margin_left, y_rx, width - margin_right, y_rx);
        tx_d = format!("M {:.1},{:.1} L {:.1},{:.1}", margin_left, y_tx, width - margin_right, y_tx);
    } else {
        for (i, (x, y)) in rx_path_points.iter().enumerate() {
            if i == 0 {
                rx_d.push_str(&format!("M {:.1},{:.1}", x, y));
            } else {
                rx_d.push_str(&format!(" L {:.1},{:.1}", x, y));
            }
        }
        for (i, (x, y)) in tx_path_points.iter().enumerate() {
            if i == 0 {
                tx_d.push_str(&format!("M {:.1},{:.1}", x, y));
            } else {
                tx_d.push_str(&format!(" L {:.1},{:.1}", x, y));
            }
        }
    }

    let safe_title = escape_xml(title);
    let safe_name = escape_xml(peer_name);

    let mut out = String::new();
    out.push_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n", width, height, width, height));
    out.push_str(&format!("  <rect width=\"{}\" height=\"{}\" rx=\"16\" fill=\"#ffffff\"/>\n", width, height));
    out.push_str(&format!("  <text x=\"{}\" y=\"36\" font-family=\"Vazirmatn\" font-size=\"18\" font-weight=\"bold\" fill=\"#111827\">{}</text>\n", margin_left, safe_title));
    out.push_str(&format!("  <text x=\"{}\" y=\"56\" font-family=\"Vazirmatn\" font-size=\"13\" fill=\"#6b7280\">{}</text>\n", margin_left, safe_name));
    out.push_str(&format!("  <rect x=\"{}\" y=\"24\" width=\"120\" height=\"28\" rx=\"14\" fill=\"#eef2ff\"/>\n", width - margin_right - 120.0));
    out.push_str(&format!("  <text x=\"{}\" y=\"43\" font-family=\"Vazirmatn\" font-size=\"12\" font-weight=\"bold\" fill=\"#4338ca\" text-anchor=\"middle\">Total: {}</text>\n", width - margin_right - 60.0, fmt_bytes(total_combined)));

    out.push_str(&format!("  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e5e7eb\" stroke-dasharray=\"4\"/>\n", margin_left, margin_top, width - margin_right, margin_top));
    out.push_str(&format!("  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e5e7eb\" stroke-dasharray=\"4\"/>\n", margin_left, margin_top + chart_h / 2.0, width - margin_right, margin_top + chart_h / 2.0));
    out.push_str(&format!("  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e5e7eb\"/>\n", margin_left, margin_top + chart_h, width - margin_right, margin_top + chart_h));

    out.push_str(&format!("  <path d=\"{}\" fill=\"none\" stroke=\"#2563eb\" stroke-width=\"2.5\"/>\n", rx_d));
    out.push_str(&format!("  <path d=\"{}\" fill=\"none\" stroke=\"#10b981\" stroke-width=\"2.5\"/>\n", tx_d));

    let legend_y = height - 20.0;
    let legend_text_y = height - 16.0;
    out.push_str(&format!("  <circle cx=\"{}\" cy=\"{}\" r=\"4\" fill=\"#2563eb\"/>\n", margin_left, legend_y));
    out.push_str(&format!("  <text x=\"{}\" y=\"{}\" font-family=\"Vazirmatn\" font-size=\"12\" fill=\"#374151\">RX: {}</text>\n", margin_left + 10.0, legend_text_y, fmt_bytes(rx_total)));
    out.push_str(&format!("  <circle cx=\"{}\" cy=\"{}\" r=\"4\" fill=\"#10b981\"/>\n", margin_left + 140.0, legend_y));
    out.push_str(&format!("  <text x=\"{}\" y=\"{}\" font-family=\"Vazirmatn\" font-size=\"12\" fill=\"#374151\">TX: {}</text>\n", margin_left + 150.0, legend_text_y, fmt_bytes(tx_total)));
    out.push_str("</svg>\n");

    out
}

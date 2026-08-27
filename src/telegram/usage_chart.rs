use super::svg_render::fmt_bytes;

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&apos;")
}

pub fn nice_ticks(max_value: f64, tick_count: usize) -> Vec<f64> {
    if max_value <= 0.0 {
        return vec![0.0, 1.0];
    }
    let count = tick_count.max(2);
    let raw_step = max_value / (count - 1) as f64;
    let mag = 10f64.powf(raw_step.log10().floor());
    let mut step = mag;
    for mult in [1.0, 2.0, 2.5, 5.0, 10.0] {
        step = mult * mag;
        if step >= raw_step {
            break;
        }
    }
    let top = (max_value / step).ceil() * step;
    let n = (top / step).round() as usize;
    (0..=n).map(|i| i as f64 * step).collect()
}

pub fn monotone_path(points: &[(f64, f64)]) -> String {
    let n = points.len();
    if n == 0 {
        return String::new();
    }
    if n == 1 {
        return format!("M {:.2},{:.2}", points[0].0, points[0].1);
    }
    let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
    let mut dx = Vec::with_capacity(n - 1);
    let mut slopes = Vec::with_capacity(n - 1);
    for i in 0..(n - 1) {
        let d = xs[i + 1] - xs[i];
        dx.push(d);
        slopes.push(if d.abs() > 1e-6 { (ys[i + 1] - ys[i]) / d } else { 0.0 });
    }
    let mut m = vec![0.0; n];
    m[0] = slopes[0];
    m[n - 1] = slopes[n - 2];
    for i in 1..(n - 1) {
        if slopes[i - 1] * slopes[i] <= 0.0 {
            m[i] = 0.0;
        } else {
            let w1 = 2.0 * dx[i] + dx[i - 1];
            let w2 = dx[i] + 2.0 * dx[i - 1];
            m[i] = (w1 + w2) / (w1 / slopes[i - 1] + w2 / slopes[i]);
        }
    }
    let mut d = format!("M {:.2},{:.2}", xs[0], ys[0]);
    for i in 0..(n - 1) {
        let h = dx[i];
        let c1x = xs[i] + h / 3.0;
        let c1y = ys[i] + m[i] * h / 3.0;
        let c2x = xs[i + 1] - h / 3.0;
        let c2y = ys[i + 1] - m[i + 1] * h / 3.0;
        d.push_str(&format!(" C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}", c1x, c1y, c2x, c2y, xs[i + 1], ys[i + 1]));
    }
    d
}

pub fn generate_usage_chart_svg(
    title: &str,
    peer_name: &str,
    rx_total: i64,
    tx_total: i64,
    points: &[(String, i64, i64)],
) -> String {
    let width = 600.0;
    let height = 320.0;
    let plot_l = 65.0;
    let plot_r = 570.0;
    let plot_t = 80.0;
    let plot_b = 240.0;
    let plot_w = plot_r - plot_l;
    let plot_h = plot_b - plot_t;

    let total_combined = rx_total + tx_total;
    let safe_title = escape_xml(title);
    let safe_name = escape_xml(peer_name);

    let max_rx = points.iter().map(|p| p.1).max().unwrap_or(0);
    let max_tx = points.iter().map(|p| p.2).max().unwrap_or(0);
    let raw_max_bytes = max_rx.max(max_tx).max(1);

    // Use MB units for nice axis ticks
    let mib = 1024.0 * 1024.0;
    let max_mb = (raw_max_bytes as f64 / mib).max(1.0);
    let mb_ticks = nice_ticks(max_mb, 5);
    let y_max_bytes = mb_ticks.last().copied().unwrap_or(max_mb) * mib;

    let mut out = String::with_capacity(4096);
    out.push_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\">\n", width, height, width, height));
    
    // Gradients
    out.push_str("  <defs>\n");
    out.push_str("    <linearGradient id=\"rx-grad\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\n");
    out.push_str("      <stop offset=\"0%\" stop-color=\"#2563eb\" stop-opacity=\"0.28\"/>\n");
    out.push_str("      <stop offset=\"100%\" stop-color=\"#2563eb\" stop-opacity=\"0.0\"/>\n");
    out.push_str("    </linearGradient>\n");
    out.push_str("    <linearGradient id=\"tx-grad\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\n");
    out.push_str("      <stop offset=\"0%\" stop-color=\"#10b981\" stop-opacity=\"0.28\"/>\n");
    out.push_str("      <stop offset=\"100%\" stop-color=\"#10b981\" stop-opacity=\"0.0\"/>\n");
    out.push_str("    </linearGradient>\n");
    out.push_str("  </defs>\n");

    // Card background
    out.push_str(&format!("  <rect width=\"{:.0}\" height=\"{:.0}\" rx=\"16\" fill=\"#ffffff\"/>\n", width, height));

    // Header: Title & Peer Name
    out.push_str(&format!("  <text x=\"{:.0}\" y=\"36\" font-family=\"Vazirmatn\" font-size=\"17\" font-weight=\"bold\" fill=\"#111827\">{}</text>\n", plot_l, safe_title));
    out.push_str(&format!("  <text x=\"{:.0}\" y=\"56\" font-family=\"Vazirmatn\" font-size=\"13\" fill=\"#6b7280\">{}</text>\n", plot_l, safe_name));

    // Total Badge
    let total_str = fmt_bytes(total_combined);
    let badge_w = 120.0;
    out.push_str(&format!("  <rect x=\"{:.0}\" y=\"24\" width=\"{:.0}\" height=\"28\" rx=\"14\" fill=\"#eef2ff\"/>\n", plot_r - badge_w, badge_w));
    out.push_str(&format!("  <text x=\"{:.0}\" y=\"43\" font-family=\"Vazirmatn\" font-size=\"12\" font-weight=\"bold\" fill=\"#4338ca\" text-anchor=\"middle\">Total: {}</text>\n", plot_r - (badge_w / 2.0), total_str));

    // Horizontal grid lines and Y-axis tick labels
    for &tick_mb in &mb_ticks {
        let tick_bytes = tick_mb * mib;
        let y = plot_b - (tick_bytes / y_max_bytes) * plot_h;
        let is_base = (tick_mb - 0.0).abs() < 1e-6;
        let stroke_color = if is_base { "#e5e7eb" } else { "#f3f4f6" };
        let stroke_dash = if is_base { "" } else { " stroke-dasharray=\"3,3\"" };
        
        out.push_str(&format!("  <line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\"{stroke_dash}/>\n", plot_l, y, plot_r, y, stroke_color));

        let tick_label = if tick_mb >= 1024.0 {
            format!("{:.1} GB", tick_mb / 1024.0)
        } else if tick_mb >= 1.0 || tick_mb == 0.0 {
            format!("{:.0} MB", tick_mb)
        } else {
            format!("{:.0} KB", tick_mb * 1024.0)
        };
        out.push_str(&format!("  <text x=\"{:.1}\" y=\"{:.1}\" font-family=\"Vazirmatn\" font-size=\"11\" fill=\"#9ca3af\" text-anchor=\"end\">{}</text>\n", plot_l - 8.0, y + 4.0, tick_label));
    }

    let n = points.len();
    if n == 0 || (total_combined == 0 && points.iter().all(|p| p.1 == 0 && p.2 == 0)) {
        // Empty state
        out.push_str(&format!("  <text x=\"{:.1}\" y=\"{:.1}\" font-family=\"Vazirmatn\" font-size=\"13\" fill=\"#9ca3af\" text-anchor=\"middle\">No traffic recorded for this period</text>\n", plot_l + plot_w / 2.0, plot_t + plot_h / 2.0));
    } else {
        // Calculate coordinates for points
        let sx = |i: usize| -> f64 {
            if n <= 1 {
                plot_l + plot_w / 2.0
            } else {
                plot_l + (i as f64 / (n - 1) as f64) * plot_w
            }
        };

        let sy = |val: i64| -> f64 {
            let clamped = (val as f64).max(0.0);
            plot_b - (clamped / y_max_bytes) * plot_h
        };

        let rx_coords: Vec<(f64, f64)> = points.iter().enumerate().map(|(i, p)| (sx(i), sy(p.1))).collect();
        let tx_coords: Vec<(f64, f64)> = points.iter().enumerate().map(|(i, p)| (sx(i), sy(p.2))).collect();

        // X-axis labels: show up to 7 evenly spaced labels
        let step = if n <= 7 { 1 } else { (n as f64 / 6.0).ceil() as usize };
        for i in (0..n).step_by(step) {
            let (label, _, _) = &points[i];
            let x = sx(i);
            out.push_str(&format!("  <text x=\"{:.1}\" y=\"{:.1}\" font-family=\"Vazirmatn\" font-size=\"11\" fill=\"#6b7280\" text-anchor=\"middle\">{}</text>\n", x, plot_b + 18.0, escape_xml(label)));
        }
        if (n - 1) % step != 0 && n > 1 {
            let (label, _, _) = &points[n - 1];
            let x = sx(n - 1);
            out.push_str(&format!("  <text x=\"{:.1}\" y=\"{:.1}\" font-family=\"Vazirmatn\" font-size=\"11\" fill=\"#6b7280\" text-anchor=\"middle\">{}</text>\n", x, plot_b + 18.0, escape_xml(label)));
        }

        // Draw Areas and Lines
        let rx_curve = monotone_path(&rx_coords);
        let tx_curve = monotone_path(&tx_coords);

        let first_x = rx_coords.first().map(|p| p.0).unwrap_or(plot_l);
        let last_x = rx_coords.last().map(|p| p.0).unwrap_or(plot_r);

        // RX (Download) Area and Line
        if !rx_curve.is_empty() {
            let rx_area = format!("{} L {:.2},{:.2} L {:.2},{:.2} Z", rx_curve, last_x, plot_b, first_x, plot_b);
            out.push_str(&format!("  <path d=\"{}\" fill=\"url(#rx-grad)\"/>\n", rx_area));
            out.push_str(&format!("  <path d=\"{}\" fill=\"none\" stroke=\"#2563eb\" stroke-width=\"2.5\" stroke-linejoin=\"round\"/>\n", rx_curve));
        }

        // TX (Upload) Area and Line
        if !tx_curve.is_empty() {
            let tx_area = format!("{} L {:.2},{:.2} L {:.2},{:.2} Z", tx_curve, last_x, plot_b, first_x, plot_b);
            out.push_str(&format!("  <path d=\"{}\" fill=\"url(#tx-grad)\"/>\n", tx_area));
            out.push_str(&format!("  <path d=\"{}\" fill=\"none\" stroke=\"#10b981\" stroke-width=\"2.5\" stroke-linejoin=\"round\"/>\n", tx_curve));
        }

        // Small data point dots if points <= 31
        if n <= 31 {
            for (x, y) in &rx_coords {
                if *y < plot_b - 1.0 {
                    out.push_str(&format!("  <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.5\" fill=\"#2563eb\"/>\n", x, y));
                }
            }
            for (x, y) in &tx_coords {
                if *y < plot_b - 1.0 {
                    out.push_str(&format!("  <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.5\" fill=\"#10b981\"/>\n", x, y));
                }
            }
        }
    }

    // Legend at bottom
    let legend_y = height - 20.0;
    out.push_str(&format!("  <circle cx=\"160\" cy=\"{:.1}\" r=\"4.5\" fill=\"#2563eb\"/>\n", legend_y - 4.0));
    out.push_str(&format!("  <text x=\"172\" y=\"{:.1}\" font-family=\"Vazirmatn\" font-size=\"12\" fill=\"#374151\">Download (RX): <b>{}</b></text>\n", legend_y, fmt_bytes(rx_total)));

    out.push_str(&format!("  <circle cx=\"370\" cy=\"{:.1}\" r=\"4.5\" fill=\"#10b981\"/>\n", legend_y - 4.0));
    out.push_str(&format!("  <text x=\"382\" y=\"{:.1}\" font-family=\"Vazirmatn\" font-size=\"12\" fill=\"#374151\">Upload (TX): <b>{}</b></text>\n", legend_y, fmt_bytes(tx_total)));

    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::svg_render::render_svg_to_png;

    #[test]
    fn test_nice_ticks_and_monotone_path() {
        let ticks = nice_ticks(45.0, 5);
        assert!(!ticks.is_empty());
        assert_eq!(ticks[0], 0.0);
        assert!(ticks.last().copied().unwrap() >= 45.0);

        let pts = vec![(0.0, 100.0), (50.0, 80.0), (100.0, 20.0), (150.0, 60.0)];
        let path = monotone_path(&pts);
        assert!(path.starts_with("M 0.00,100.00"));
        assert!(path.contains("C "));
    }

    #[test]
    fn test_generate_and_render_usage_chart() {
        let points = vec![
            ("00:00".to_string(), 1024 * 1024 * 5, 1024 * 1024 * 2),
            ("04:00".to_string(), 1024 * 1024 * 15, 1024 * 1024 * 8),
            ("08:00".to_string(), 1024 * 1024 * 40, 1024 * 1024 * 12),
            ("12:00".to_string(), 1024 * 1024 * 85, 1024 * 1024 * 25),
            ("16:00".to_string(), 1024 * 1024 * 60, 1024 * 1024 * 18),
            ("20:00".to_string(), 1024 * 1024 * 110, 1024 * 1024 * 35),
        ];

        let svg = generate_usage_chart_svg("Today's Usage", "peer5", 1024 * 1024 * 315, 1024 * 1024 * 100, &points);
        assert!(svg.contains("Today&apos;s Usage") || svg.contains("Today's Usage"));
        assert!(svg.contains("peer5"));
        assert!(svg.contains("rx-grad"));

        let png_res = render_svg_to_png(&svg, 2.0);
        assert!(png_res.is_ok(), "PNG rendering failed: {:?}", png_res.err());
        let png_bytes = png_res.unwrap();
        assert!(!png_bytes.is_empty());
        assert_eq!(&png_bytes[0..4], &[0x89, b'P', b'N', b'G']);
    }
}

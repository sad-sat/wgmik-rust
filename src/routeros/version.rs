pub const MIN_ROUTEROS_VERSION: (i32, i32) = (7, 15);

pub fn parse_routeros_version(value: Option<&str>) -> Option<(i32, i32, i32)> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let major = parts[0].parse::<i32>().ok()?;
    // Handle versions like "15.1rc1" or "15"
    let minor_str = parts[1].chars().take_while(|c| c.is_ascii_digit()).collect::<String>();
    let minor = minor_str.parse::<i32>().ok()?;
    let patch = parts.get(2).and_then(|p| {
        let p_str = p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>();
        p_str.parse::<i32>().ok()
    }).unwrap_or(0);

    Some((major, minor, patch))
}

pub fn is_routeros_supported(value: Option<&str>) -> bool {
    match parse_routeros_version(value) {
        Some((major, minor, _)) => (major, minor) >= MIN_ROUTEROS_VERSION,
        None => false,
    }
}

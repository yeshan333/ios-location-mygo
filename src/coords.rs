use std::f64::consts::PI;

const EARTH_A: f64 = 6378245.0;
const EARTH_EE: f64 = 0.00669342162296594323;
pub const TILE_SIZE: f64 = 256.0;

pub fn out_of_china(lat: f64, lng: f64) -> bool {
    !(72.004..=137.8347).contains(&lng) || !(0.8293..=55.8271).contains(&lat)
}

fn transform_lat(x: f64, y: f64) -> f64 {
    let mut r = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * x.abs().sqrt();
    r += (20.0 * (6.0 * x * PI).sin() + 20.0 * (2.0 * x * PI).sin()) * 2.0 / 3.0;
    r += (20.0 * (y * PI).sin() + 40.0 * (y / 3.0 * PI).sin()) * 2.0 / 3.0;
    r += (160.0 * (y / 12.0 * PI).sin() + 320.0 * (y * PI / 30.0).sin()) * 2.0 / 3.0;
    r
}

fn transform_lng(x: f64, y: f64) -> f64 {
    let mut r = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * x.abs().sqrt();
    r += (20.0 * (6.0 * x * PI).sin() + 20.0 * (2.0 * x * PI).sin()) * 2.0 / 3.0;
    r += (20.0 * (x * PI).sin() + 40.0 * (x / 3.0 * PI).sin()) * 2.0 / 3.0;
    r += (150.0 * (x / 12.0 * PI).sin() + 300.0 * (x / 30.0 * PI).sin()) * 2.0 / 3.0;
    r
}

/// Convert WGS-84 coordinates to GCJ-02 (China Mars datum) for Google/AutoNavi China tiles.
pub fn wgs_to_gcj(lat: f64, lng: f64) -> (f64, f64) {
    if out_of_china(lat, lng) {
        return (lat, lng);
    }
    let mut d_lat = transform_lat(lng - 105.0, lat - 35.0);
    let mut d_lng = transform_lng(lng - 105.0, lat - 35.0);
    let rad_lat = lat / 180.0 * PI;
    let mut magic = rad_lat.sin();
    magic = 1.0 - EARTH_EE * magic * magic;
    let sqrt_magic = magic.sqrt();
    d_lat = (d_lat * 180.0) / ((EARTH_A * (1.0 - EARTH_EE)) / (magic * sqrt_magic) * PI);
    d_lng = (d_lng * 180.0) / (EARTH_A / sqrt_magic * rad_lat.cos() * PI);
    (lat + d_lat, lng + d_lng)
}

/// Convert GCJ-02 coordinates back to WGS-84.
pub fn gcj_to_wgs(lat: f64, lng: f64) -> (f64, f64) {
    if out_of_china(lat, lng) {
        return (lat, lng);
    }
    let (g_lat, g_lng) = wgs_to_gcj(lat, lng);
    (lat * 2.0 - g_lat, lng * 2.0 - g_lng)
}

/// Clamp latitude to Web Mercator valid range [-85.05112878, 85.05112878].
pub fn clamp_mercator_lat(lat: f64) -> f64 {
    lat.clamp(-85.05112878, 85.05112878)
}

/// Wrap longitude into [-180, 180].
pub fn wrap_lng(lng: f64) -> f64 {
    let mut x = (lng + 180.0) % 360.0;
    if x < 0.0 {
        x += 360.0;
    }
    x - 180.0
}

/// Project (lat, lng) to Web Mercator world pixel coordinates at fractional zoom `zoom`.
pub fn lat_lng_to_world_px(lat: f64, lng: f64, zoom: f64) -> (f64, f64) {
    let scale = TILE_SIZE * 2.0_f64.powf(zoom);
    let x = (lng + 180.0) / 360.0 * scale;
    let sin_lat = clamp_mercator_lat(lat).to_radians().sin();
    let y = (0.5 - ((1.0 + sin_lat) / (1.0 - sin_lat)).ln() / (4.0 * PI)) * scale;
    (x, y)
}

/// Unproject Web Mercator world pixel coordinates back to (lat, lng).
pub fn world_px_to_lat_lng(x: f64, y: f64, zoom: f64) -> (f64, f64) {
    let scale = TILE_SIZE * 2.0_f64.powf(zoom);
    let lng = wrap_lng(x / scale * 360.0 - 180.0);
    let n = PI - 2.0 * PI * (y / scale);
    let lat = clamp_mercator_lat(n.sinh().atan().to_degrees());
    (lat, lng)
}

pub fn validate_coords(lat: f64, lng: f64) -> Result<(), String> {
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        return Err(format!("Latitude out of range: {lat}"));
    }
    if !lng.is_finite() || !(-180.0..=180.0).contains(&lng) {
        return Err(format!("Longitude out of range: {lng}"));
    }
    Ok(())
}

/// Parse direct coordinate strings like `"35.65858, 139.74543"` or `"34.0537 -118.2428"`.
pub fn parse_coord_query(q: &str) -> Option<(f64, f64)> {
    let trimmed = q.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = if trimmed.contains(',') {
        trimmed.split(',').map(str::trim).filter(|s| !s.is_empty()).collect()
    } else {
        trimmed.split_whitespace().collect()
    };
    if parts.len() != 2 {
        return None;
    }
    let lat: f64 = parts[0].parse().ok()?;
    let lng: f64 = parts[1].parse().ok()?;
    validate_coords(lat, lng).ok()?;
    Some((lat, lng))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_coords() {
        assert!(validate_coords(34.0537, -118.2428).is_ok());
        assert!(validate_coords(91.0, 0.0).is_err());
        assert!(validate_coords(0.0, -181.0).is_err());
    }

    #[test]
    fn test_parse_coord_query() {
        assert_eq!(parse_coord_query("35.65858, 139.74543"), Some((35.65858, 139.74543)));
        assert_eq!(parse_coord_query("34.0537 -118.2428"), Some((34.0537, -118.2428)));
        assert_eq!(parse_coord_query("Tokyo Tower"), None);
    }

    #[test]
    fn test_gcj_roundtrip_china_and_overseas() {
        // Overseas should be identity
        assert_eq!(wgs_to_gcj(34.0537, -118.2428), (34.0537, -118.2428));
        // Beijing should offset and round-trip within < 1e-4 deg
        let (glat, glng) = wgs_to_gcj(39.90872, 116.39748);
        assert!((glat - 39.90872).abs() > 1e-4);
        let (wlat, wlng) = gcj_to_wgs(glat, glng);
        assert!((wlat - 39.90872).abs() < 1e-4);
        assert!((wlng - 116.39748).abs() < 1e-4);
    }

    #[test]
    fn test_mercator_roundtrip() {
        let (x, y) = lat_lng_to_world_px(34.0536923, -118.2427676, 13.0);
        let (lat, lng) = world_px_to_lat_lng(x, y, 13.0);
        assert!((lat - 34.0536923).abs() < 1e-6);
        assert!((lng - -118.2427676).abs() < 1e-6);
    }
}

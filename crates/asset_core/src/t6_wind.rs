//! bo2mp: Black Ops II's global wind, as the client builds it for the tree
//! canopy shaders (CG_UpdateWind, read from the client exe; values from each
//! map's script): the map's wind vector, sixteen wind variants seeded from
//! it, and each variant's rustle at a time.

/// The wind vector a map's script sets (`wind_global_vector`); the dvar's
/// default for a map that sets none.
pub fn t6_map_wind(zone: &str) -> [f32; 3] {
    let name = zone.rsplit(['/', '\\']).next().unwrap_or(zone);
    let name = name.strip_suffix(".ff").unwrap_or(name);
    let name = name.strip_prefix("mp_").unwrap_or(name);
    match name {
        "la" => [-80.0, 115.0, -165.0],
        "socotra" => [-150.0, -145.0, -150.0],
        "dockside" | "carrier" | "nightclub" | "slums" | "drone" | "express" | "hijacked"
        | "meltdown" | "raid" | "village" => [-120.0, -115.0, -120.0],
        "overflow" => [-130.0, -130.0, -130.0],
        "turbine" => [-200.0, -230.0, -200.0],
        "nuketown_2020" => [-135.0, -135.0, -135.0],
        "mirage" | "downhill" => [-145.0, -145.0, -145.0],
        "skate" => [-145.0, -175.0, -160.0],
        "hydro" => [1.0, 0.0, 0.0],
        "studio" => [-160.0, -149.0, 160.0],
        "vertigo" => [197.0, -215.0, 95.0],
        "magma" => [-125.0, -115.0, -125.0],
        "concert" => [-125.0, -125.0, -125.0],
        "uplink" => [-155.0, -155.0, -155.0],
        "castaway" => [-55.0, -120.0, -110.0],
        "takeoff" => [-210.0, -210.0, -210.0],
        "pod" => [-90.0, -140.0, -95.0],
        "dig" => [-105.0, -105.0, -105.0],
        _ => [-110.0, -150.0, -110.0],
    }
}

/// The canopy shaders' wind table at `seconds` of game time: first
/// windDirection (the vector's direction on the ground, then its length),
/// then the sixteen variantWindSprings (0, then each axis's rustle in
/// radians). The variants are seeded as the game seeds them, by the C
/// runtime's rand from 0x7fff: seventeen jitters of a tenth either way,
/// then for each variant (after one draw the shader never reads) the
/// vector jittered three ways, its strength (squared length over 176
/// squared), and three amplitudes and three frequencies scaled by it.
pub fn t6_wind_table(wind: [f32; 3], seconds: f32) -> [[f32; 4]; 17] {
    const UNIT: f32 = 3.051_851e-5;
    let mut state: u32 = 0x7fff;
    let mut rand = move || {
        state = state.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((state >> 16) & 0x7fff) as f32 - 16383.0
    };
    let mut jitter = [0.0f32; 17];
    for j in &mut jitter {
        *j = rand() * 0.1 * UNIT;
    }
    let mut table = [[0.0f32; 4]; 17];
    let ground = (wind[0] * wind[0] + wind[1] * wind[1]).sqrt();
    let length = (wind[0] * wind[0] + wind[1] * wind[1] + wind[2] * wind[2]).sqrt();
    if ground > 0.0 {
        table[0] = [wind[0] / ground, wind[1] / ground, 0.0, length];
    }
    let turn = (f64::from(seconds) % 3600.0) as f32;
    for i in 0..16 {
        rand();
        let v = [0, 1, 2].map(|a| (jitter[(i + a) % 17] + 1.0) * wind[a]);
        let k = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) / (176.0 * 176.0);
        let amp = [rand(), rand(), rand()].map(|r| r * UNIT * k);
        let freq = [rand(), rand(), rand()].map(|r| k * std::f32::consts::PI * r * UNIT);
        let row = &mut table[1 + i];
        for a in 0..3 {
            let phase = (freq[a] * turn) % std::f32::consts::TAU;
            row[1 + a] = phase.sin() * amp[a];
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_names() {
        assert_eq!(t6_map_wind("mp_castaway"), [-55.0, -120.0, -110.0]);
        assert_eq!(t6_map_wind("mp_nuketown_2020"), [-135.0, -135.0, -135.0]);
        assert_eq!(t6_map_wind("mp_bridge"), [-110.0, -150.0, -110.0]);
    }

    #[test]
    fn rustle_moves_and_stays_small() {
        let w = t6_map_wind("mp_castaway");
        let a = t6_wind_table(w, 10.0);
        let b = t6_wind_table(w, 10.5);
        assert!((a[0][0] * a[0][0] + a[0][1] * a[0][1] - 1.0).abs() < 1e-5);
        assert!((1..17).any(|i| (a[i][1] - b[i][1]).abs() > 1e-3));
        // Each rustle is within its strength (under ~1.2 for this map).
        assert!(a.iter().skip(1).all(|r| r[1..].iter().all(|x| x.abs() < 1.2)));
    }
}

//! Verteilung der Finger und Schlitze auf einer Kante.
//!
//! Die Kante wird in eine ungerade Zahl gleich breiter Abschnitte geteilt. Finger liegen auf den
//! geraden Abschnitten (0, 2, 4, …), Schlitze auf den ungeraden (1, 3, 5, …). Eine ungerade Anzahl
//! sieht vom anderen Ende gleich aus, deshalb treffen sich gleich indizierte Abschnitte immer,
//! egal welches Kantenende an der Ecke liegt. Die Schlitze sitzen damit in den Lücken der Finger
//! und sind um eine Nennbreite versetzt. Beide Enden sind bei den Fingern ein Finger und bei den
//! Schlitzen Material.
//!
//! Ohne vorgegebene Anzahl bleibt jeder Abschnitt so breit wie die Nennbreite (automatisch das
//! 2,5-fache der Materialstärke). Was übrig bleibt, liegt zu gleichen Teilen an beiden Enden, damit
//! die Ecken frei bleiben. Eine vorgegebene Anzahl oder eine gekappte sehr lange Kante streckt die
//! Abschnitte stattdessen gleichmäßig über die nutzbare Länge.
//!
//! Kerf und Pressung ändern nicht die Mitten, nur die Breite. Der Laser trägt auf jeder Flanke
//! Material ab: ein Finger wird um `kerf/2 + fit` breiter modelliert, ein Schlitz um denselben
//! Betrag schmaler. Dazu kommt `fit_percent` der Nennbreite. Die Lücke zwischen zwei Fingern wird
//! um denselben Betrag enger, die Verbindung bleibt zentrisch.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fingers,
    Slots,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub length: f64,
    pub thickness: f64,
    pub margin: f64,
    pub finger_width: Option<f64>,
    pub finger_count: Option<u32>,
    pub kerf: f64,
    pub fit: f64,
    pub fit_percent: f64,
    pub mode: Mode,
}

#[derive(Clone, Debug)]
pub struct Feature {
    /// Abschnitt auf der Kante. Finger: 0, 2, 4, … Schlitze: 1, 3, 5, …
    #[allow(dead_code)]
    pub segment: u32,
    /// Abstand vom kanonischen Kantenanfang bis zur Mitte des Features.
    #[allow(dead_code)]
    pub center: f64,
    /// Faktor in `Länge/2 + Faktor * Nennbreite`.
    pub coeff: f64,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub segments: u32,
    pub nominal: f64,
    pub adjust: f64,
    pub feature_width: f64,
    pub end_gap: f64,
    pub features: Vec<Feature>,
    /// Die natürliche Anzahl hätte das Limit überschritten, die Finger sind deshalb breiter.
    pub widened: bool,
}

const MAX_SEGMENTS: u32 = 31;

pub fn plan(spec: &Spec) -> Result<Plan, String> {
    if !spec.length.is_finite() || spec.length <= 0.0 {
        return Err("Kantenlänge ist nicht positiv".into());
    }
    if !spec.thickness.is_finite() || spec.thickness < 0.15 || spec.thickness > 80.0 {
        return Err(format!(
            "Materialstärke {:.3} mm liegt außerhalb von 0,15 bis 80 mm",
            spec.thickness
        ));
    }
    if !spec.kerf.is_finite() || spec.kerf < 0.0 || spec.kerf > 5.0 {
        return Err(format!(
            "Kerf {:.3} mm muss zwischen 0 und 5 mm liegen",
            spec.kerf
        ));
    }
    if !spec.fit.is_finite() || spec.fit < -2.0 || spec.fit > 2.0 {
        return Err(format!(
            "Pressung {:.3} mm muss zwischen -2 und 2 mm liegen",
            spec.fit
        ));
    }
    if !spec.fit_percent.is_finite() || spec.fit_percent < -20.0 || spec.fit_percent > 20.0 {
        return Err(format!(
            "Pressung {:.2} % muss zwischen -20 und 20 liegen",
            spec.fit_percent
        ));
    }
    if !spec.margin.is_finite() || spec.margin < 0.0 {
        return Err("Eckeneinzug darf nicht negativ sein".into());
    }
    if spec.margin * 2.0 >= spec.length - 0.5 {
        return Err(format!(
            "Eckeneinzug {:.2} mm auf beiden Seiten lässt von der {:.2} mm langen Kante nichts übrig",
            spec.margin, spec.length
        ));
    }

    let target = match spec.finger_width {
        Some(w) if w > 0.0 => round_mm(w),
        Some(_) => {
            return Err("Fingerbreite muss größer als 0 sein, oder 0 für automatisch".into());
        }
        None => round_mm(2.5 * spec.thickness),
    };
    if target <= spec.thickness {
        return Err(format!(
            "Fingerbreite {target:.3} mm muss größer sein als die Materialstärke {:.3} mm",
            spec.thickness
        ));
    }

    let usable = spec.length - 2.0 * spec.margin;
    let (segments, nominal, widened) = choose_count(spec, target, usable)?;
    let adjust = spec.kerf / 2.0 + spec.fit + nominal * spec.fit_percent / 100.0;
    let finger_w = nominal + adjust;
    let slot_w = nominal - adjust;
    // Beide Breiten müssen stehen bleiben, auch die der nicht gebauten Gegenseite: sonst frisst
    // die Kompensation die Lücke oder schließt den Schlitz.
    if finger_w <= spec.thickness || slot_w <= 0.3 {
        return Err(format!(
            "Kerf {:.3} mm und Pressung {:.3} mm lassen bei {:.3} mm Nennbreite keinen offenen Schlitz (Finger {:.3} mm, Schlitz {:.3} mm)",
            spec.kerf, spec.fit, nominal, finger_w, slot_w
        ));
    }
    let feature_width = match spec.mode {
        Mode::Fingers => finger_w,
        Mode::Slots => slot_w,
    };
    let end_gap = (spec.length - segments as f64 * nominal) / 2.0;
    let overhang = (feature_width - nominal) / 2.0;
    if overhang > end_gap + 0.25 {
        return Err(
            "Kerf und Pressung lassen die Finger über das Kantenende ragen. Vergrößere den Eckeneinzug oder verringere Kerf und Pressung."
                .into(),
        );
    }

    // Schlitze um eine Nennbreite versetzt, in die Lücken der Finger. Bei ungerader
    // Abschnittszahl bleibt diese Phasenlage auch dann gültig, wenn die Gegenkante
    // vom anderen Ende her gezählt wird.
    let mut features = Vec::new();
    let mut i = match spec.mode {
        Mode::Fingers => 0,
        Mode::Slots => 1,
    };
    while i < segments {
        let coeff = i as f64 + 0.5 - segments as f64 / 2.0;
        features.push(Feature {
            segment: i,
            center: spec.length / 2.0 + coeff * nominal,
            coeff,
        });
        i += 2;
    }
    if features.is_empty() {
        return Err(
            "für Schlitze braucht die Kante mindestens zwei Abschnitte, damit sie in den Lücken zwischen den Fingern liegen. Verringere die Fingerbreite oder erhöhe die Anzahl."
                .into(),
        );
    }

    Ok(Plan {
        segments,
        nominal,
        adjust,
        feature_width,
        end_gap,
        features,
        widened,
    })
}

fn choose_count(spec: &Spec, target: f64, usable: f64) -> Result<(u32, f64, bool), String> {
    if usable <= target {
        return Err(too_short(spec, target));
    }
    if let Some(n) = spec.finger_count {
        if n == 0 || n > MAX_SEGMENTS {
            return Err(format!(
                "Anzahl muss zwischen 1 und {MAX_SEGMENTS} liegen"
            ));
        }
        let nominal = round_mm(usable / n as f64);
        if nominal <= spec.thickness {
            return Err(format!(
                "Anzahl {n} macht die Finger {nominal:.3} mm breit, schmaler als die Materialstärke {:.3} mm",
                spec.thickness
            ));
        }
        return Ok((n, nominal, (nominal - target).abs() > 0.05));
    }

    let mut n = (usable / target).floor() as u32;
    if n == 0 {
        return Err(too_short(spec, target));
    }
    // Ungerade Anzahl: beide Enden sind Finger oder beide sind Schlitze.
    if n % 2 == 0 {
        n -= 1;
    }
    if n == 0 {
        return Err(too_short(spec, target));
    }
    if n > MAX_SEGMENTS {
        let nominal = round_mm(usable / MAX_SEGMENTS as f64);
        return Ok((MAX_SEGMENTS, nominal, true));
    }
    // Die Nennbreite bleibt. Der Rest der Kante wird auf die beiden Enden verteilt.
    if n as f64 * target > usable + 1e-6 {
        return Err(too_short(spec, target));
    }
    Ok((n, target, false))
}

fn too_short(spec: &Spec, target: f64) -> String {
    if spec.margin > 0.0 {
        format!(
            "Kante ist {:.2} mm lang und damit zu kurz für Finger von {target:.2} mm bei {:.2} mm Eckeneinzug. Setze corners auf \"flush\", verringere corner_margin oder die Fingerbreite.",
            spec.length, spec.margin
        )
    } else {
        format!(
            "Kante ist {:.2} mm lang und damit zu kurz für Finger von {target:.2} mm bei {:.2} mm Materialstärke.",
            spec.length, spec.thickness
        )
    }
}

fn round_mm(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Spec {
        Spec {
            length: 100.0,
            thickness: 3.0,
            margin: 3.0,
            finger_width: None,
            finger_count: None,
            kerf: 0.15,
            fit: 0.0,
            fit_percent: 0.0,
            mode: Mode::Slots,
        }
    }

    #[test]
    fn auto_slots_keep_the_target_width_and_park_the_remainder_at_the_ends() {
        let plan = plan(&base()).unwrap();
        assert_eq!(plan.segments, 11);
        assert_eq!(plan.features.len(), 5);
        assert!((plan.nominal - 7.5).abs() < 1e-9);
        assert!((plan.adjust - 0.075).abs() < 1e-9);
        assert!((plan.feature_width - 7.425).abs() < 1e-9);
        assert!((plan.end_gap - 8.75).abs() < 1e-9);
        assert!((plan.features[0].center - 20.0).abs() < 1e-9);
        assert!((plan.features[4].center - 80.0).abs() < 1e-9);
        assert_eq!(plan.features[0].segment, 1);
        assert_eq!(plan.features[4].segment, 9);
        assert!(!plan.widened);
    }

    #[test]
    fn slots_are_one_nominal_width_beside_the_fingers() {
        let mut fingers = base();
        fingers.mode = Mode::Fingers;
        let finger_plan = plan(&fingers).unwrap();
        let slot_plan = plan(&base()).unwrap();
        assert_eq!(finger_plan.features.len(), 6);
        assert_eq!(slot_plan.features.len(), 5);
        assert!((finger_plan.features[0].center - 12.5).abs() < 1e-9);
        assert!(
            (slot_plan.features[0].center - finger_plan.features[0].center - finger_plan.nominal).abs()
                < 1e-9
        );
        assert!(
            (slot_plan.features[4].center - finger_plan.features[5].center + finger_plan.nominal).abs()
                < 1e-9
        );
    }

    #[test]
    fn fingers_grow_by_the_same_adjustment_the_slots_shrink() {
        let mut spec = base();
        spec.mode = Mode::Fingers;
        spec.fit = 0.05;
        let plan = plan(&spec).unwrap();
        // kerf/2 + fit = 0.075 + 0.05
        assert!((plan.adjust - 0.125).abs() < 1e-9);
        assert!((plan.feature_width - 7.625).abs() < 1e-9);
    }

    #[test]
    fn fit_percent_scales_with_the_nominal_width() {
        let mut spec = base();
        spec.fit_percent = 1.0;
        let plan = plan(&spec).unwrap();
        // 0.075 + 1% von 7.5
        assert!((plan.adjust - (0.075 + 0.075)).abs() < 1e-9);
        assert!((plan.feature_width - (7.5 - 0.15)).abs() < 1e-9);
    }

    #[test]
    fn explicit_count_fills_the_usable_length() {
        let mut spec = base();
        spec.finger_count = Some(5);
        spec.mode = Mode::Fingers;
        let fingers = plan(&spec).unwrap();
        assert_eq!(fingers.segments, 5);
        assert_eq!(fingers.features.len(), 3);
        assert!((fingers.nominal - 18.8).abs() < 1e-9);
        assert!((fingers.end_gap - 3.0).abs() < 1e-6);
        assert!((fingers.features[0].center - 12.4).abs() < 1e-6);
        spec.mode = Mode::Slots;
        let slots = plan(&spec).unwrap();
        assert_eq!(slots.features.len(), 2);
        assert!((slots.features[0].center - 31.2).abs() < 1e-6);
    }

    #[test]
    fn flush_corners_use_the_whole_edge() {
        let mut spec = base();
        spec.margin = 0.0;
        let plan = plan(&spec).unwrap();
        assert_eq!(plan.segments, 13);
        assert_eq!(plan.features.len(), 6);
        assert!((plan.end_gap - 1.25).abs() < 1e-9);
    }

    #[test]
    fn one_section_cannot_hold_a_slot() {
        let mut spec = base();
        spec.finger_count = Some(1);
        spec.margin = 0.0;
        let err = plan(&spec).unwrap_err();
        assert!(err.contains("Lücken"), "{err}");
    }

    #[test]
    fn a_short_edge_names_the_inset() {
        let mut spec = base();
        spec.length = 12.0;
        let err = plan(&spec).unwrap_err();
        assert!(err.contains("flush"), "{err}");
    }

    #[test]
    fn finger_width_must_beat_the_thickness() {
        let mut spec = base();
        spec.finger_width = Some(3.0);
        let err = plan(&spec).unwrap_err();
        assert!(err.contains("Materialstärke"), "{err}");
    }

    #[test]
    fn a_large_kerf_must_not_close_the_slot() {
        let mut spec = base();
        spec.kerf = 5.0;
        spec.fit = 0.5;
        spec.finger_width = Some(3.2);
        // Nennbreite 3,2 mm liegt über der Stärke, Kerf/2 + Pressung = 3 mm schließt den Schlitz.
        let err = plan(&spec).unwrap_err();
        assert!(err.contains("Schlitz"), "{err}");
    }

    #[test]
    fn negative_kerf_is_refused() {
        let mut spec = base();
        spec.kerf = -0.1;
        assert!(plan(&spec).unwrap_err().contains("Kerf"));
    }
}

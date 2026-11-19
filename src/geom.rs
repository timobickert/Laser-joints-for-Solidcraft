//! Lage einer Plattenkante im Raum.
//!
//! Eine Laserplatte hat zwei große Flächen und einen schmalen Rand. Die gewählte Kante ist die
//! Schnittlinie aus einer großen Fläche (Platte) und einer Randfläche. Die Wandstärke ist der
//! Abstand der großen Fläche zu ihrer parallelen Gegenfläche. Finger und Schlitze liegen in der
//! Plattenebene: X läuft entlang der Kante, Y zeigt ins Material, und die Extrusion geht durch
//! die Wandstärke.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Edge {
    pub index: u32,
    pub name: Option<String>,
    pub start: [f64; 3],
    pub end: [f64; 3],
    /// Mitte der Kante. Die Auswahl trifft über Index oder Punkt, die Ebene über Anfang und Ende.
    #[allow(dead_code)]
    pub mid: [f64; 3],
    pub length: f64,
}

#[derive(Clone, Debug)]
pub struct Face {
    pub name: String,
    pub area: f64,
    pub normal: [f64; 3],
    pub point: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    /// `"positive"` extrudiert entlang X×Y, `"negative"` dagegen. Die Richtung zeigt ins Material.
    pub direction: &'static str,
    pub length: f64,
    pub stock: f64,
    pub panel_face: String,
    pub rim_face: String,
}

pub fn parse_edges(v: &Value) -> Result<Vec<Edge>, String> {
    let arr = v
        .get("edges")
        .and_then(Value::as_array)
        .ok_or("model.edges hat keine Kantenliste geliefert")?;
    arr.iter().map(parse_edge).collect()
}

pub fn parse_faces(v: &Value) -> Result<Vec<Face>, String> {
    let arr = v
        .get("faces")
        .and_then(Value::as_array)
        .ok_or("model.faces hat keine Flächenliste geliefert")?;
    let mut faces = Vec::new();
    for f in arr {
        if let Some(face) = parse_face(f)? {
            faces.push(face);
        }
    }
    Ok(faces)
}

fn parse_edge(v: &Value) -> Result<Edge, String> {
    let index = v
        .get("index")
        .and_then(Value::as_u64)
        .ok_or("Kante ohne Index")? as u32;
    let length = v
        .get("length")
        .and_then(Value::as_f64)
        .ok_or("Kante ohne Länge")?;
    let start = v
        .get("start")
        .and_then(vec3)
        .ok_or("Kante ohne Anfangspunkt")?;
    let end = v
        .get("end")
        .and_then(vec3)
        .ok_or("Kante ohne Endpunkt")?;
    let mid = v.get("mid").and_then(vec3).unwrap_or(mul(add(start, end), 0.5));
    let name = v.get("name").and_then(Value::as_str).map(str::to_string);
    Ok(Edge {
        index,
        name,
        start,
        end,
        mid,
        length,
    })
}

/// Nicht ebene Flächen haben keine Normale und taugen nicht als Platte.
fn parse_face(v: &Value) -> Result<Option<Face>, String> {
    let name = match v.get("name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => return Ok(None),
    };
    let Some(normal) = v.get("plane_normal").and_then(vec3) else {
        return Ok(None);
    };
    let Some(point) = v.get("point").and_then(vec3) else {
        return Ok(None);
    };
    let area = v.get("area").and_then(Value::as_f64).unwrap_or(0.0);
    Ok(Some(Face {
        name,
        area,
        normal,
        point,
    }))
}

pub fn frame(edge: &Edge, faces: &[Face], edges: &[Edge]) -> Result<Frame, String> {
    let straight = dist(edge.start, edge.end);
    if straight < 1e-6 {
        return Err("Kante hat keine Länge".into());
    }
    // Eine Bogenkante oder eine Schnittkurve hat eine größere Kurvenlänge als der Abstand
    // ihrer Endpunkte. Fingerzinken brauchen eine gerade Kante.
    if (edge.length - straight).abs() > 0.05_f64.max(straight * 0.002) {
        return Err(format!(
            "nur gerade Kanten; Kante {} ist gekrümmt ({:.2} mm Kurve, {:.2} mm Sehne)",
            edge.index, edge.length, straight
        ));
    }

    let (origin, far, x) = canonical(edge.start, edge.end);
    let (panel, rim) = panel_and_rim(edge, faces)?;
    let panel_n = unit(panel.normal).map_err(|_| format!("Fläche {} hat keine Normale", panel.name))?;
    let rim_n = unit(rim.normal).map_err(|_| format!("Fläche {} hat keine Normale", rim.name))?;

    // Der Körper liegt auf der Innenseite der Randfläche.
    let into = mul(rim_n, -1.0);
    let mut y = sub(into, mul(x, dot(into, x)));
    let y_len = length(y);
    if y_len < 1e-6 {
        return Err(
            "die Randfläche steht parallel zur Kante; wähle die lange Kante einer ebenen Platte"
                .into(),
        );
    }
    y = mul(y, 1.0 / y_len);

    let n = cross(x, y);
    if dot(n, panel_n).abs() < 0.98 {
        return Err(
            "die beiden Flächen an der Kante stehen nicht rechtwinklig zueinander; Fingerzinken brauchen eine ebene Platte"
                .into(),
        );
    }
    // Positive Extrusion folgt der Skizzennormale X×Y. Zeigt die nach außen, schneiden wir
    // in die Gegenrichtung, also in die Platte hinein.
    let direction = if dot(n, panel_n) >= 0.0 {
        "negative"
    } else {
        "positive"
    };

    let stock = stock_thickness(panel, panel_n, faces).or_else(|| stock_from_edges(origin, far, panel_n, edges));
    let Some(stock) = stock else {
        return Err(
            "Wandstärke nicht erkennbar. Setze material_thickness oder wähle die lange Kante einer ebenen Platte."
                .into(),
        );
    };
    if stock < 0.15 || stock > 80.0 {
        return Err(format!(
            "erkannte Wandstärke {stock:.2} mm ist für eine Laserplatte nicht plausibel"
        ));
    }
    // Die Stirnseite einer Platte ist selbst nur so lang wie die Wand. Dort würden die Zinken
    // quer durch die Schmalseite laufen.
    if straight <= stock * 2.5 {
        return Err(format!(
            "Kante {} ist nur {straight:.2} mm lang, die Wand ist {stock:.2} mm. Wähle eine lange Kante der Platte, nicht die Stirnseite.",
            edge.index
        ));
    }

    Ok(Frame {
        origin: clean3(origin),
        x: clean3(x),
        y: clean3(y),
        direction,
        length: (straight * 10_000.0).round() / 10_000.0,
        stock: (stock * 10_000.0).round() / 10_000.0,
        panel_face: panel.name.clone(),
        rim_face: rim.name.clone(),
    })
}

/// Abstand eines Punkts zur Strecke AB. Damit fällt ein Kanten-Pick auf die richtige Kante,
/// auch wenn der Index nach einem Umbau nicht mehr stimmt.
pub fn dist_point_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    if len2 < 1e-18 {
        return dist(p, a);
    }
    let t = (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0);
    dist(p, add(a, mul(ab, t)))
}

fn panel_and_rim<'a>(edge: &Edge, faces: &'a [Face]) -> Result<(&'a Face, &'a Face), String> {
    let Some(name) = edge.name.as_deref() else {
        return Err(format!(
            "Kante {} hat keinen Flächennamen; nur Kanten zwischen zwei ebenen Flächen",
            edge.index
        ));
    };
    let mut parts = name.split('|');
    let a = parts.next().unwrap_or("");
    let b = parts.next().unwrap_or("");
    if b.is_empty() || parts.next().is_some() {
        return Err(format!("Kantenname „{name}“ nennt nicht genau zwei Flächen"));
    }
    let fa = lookup_face(faces, a).ok_or_else(|| format!("Fläche {a} der Kante {} fehlt", edge.index))?;
    let fb = lookup_face(faces, b).ok_or_else(|| format!("Fläche {b} der Kante {} fehlt", edge.index))?;
    // Die große Fläche ist die Platte, die kleine der Rand. Bei gleicher Fläche (kein Blech)
    // gewinnt die größere exakt, der Fehler zeigt sich danach an der Wandstärke.
    if fa.area >= fb.area {
        Ok((fa, fb))
    } else {
        Ok((fb, fa))
    }
}

fn lookup_face<'a>(faces: &'a [Face], token: &str) -> Option<&'a Face> {
    if let Some(f) = faces.iter().find(|f| f.name == token) {
        return Some(f);
    }
    // Wiederholte Kanten heißen „FlächeA|FlächeB:2“. Die Zahl gehört zur Kante, nicht zur Fläche.
    let (head, tail) = token.rsplit_once(':')?;
    if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
        faces.iter().find(|f| f.name == head)
    } else {
        None
    }
}

/// Wandstärke = Abstand der Plattenfläche zu der parallelen Gegenfläche gleicher Größe.
/// Bei einer einfachen Platte ist das die zweite Großfläche. Eine Tasche hat eine kleinere
/// parallele Fläche; die gleich große Gegenfläche bleibt die Wand.
fn stock_thickness(panel: &Face, panel_n: [f64; 3], faces: &[Face]) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for face in faces {
        if face.name == panel.name {
            continue;
        }
        let Ok(n) = unit(face.normal) else {
            continue;
        };
        if dot(n, panel_n) > -0.995 {
            continue;
        }
        let dist = dot(sub(face.point, panel.point), panel_n).abs();
        if !(0.15..80.0).contains(&dist) {
            continue;
        }
        let area_delta = (face.area - panel.area).abs();
        let replace = match best {
            None => true,
            Some((best_delta, best_dist)) => {
                area_delta < best_delta - 1e-6 || ((area_delta - best_delta).abs() <= 1e-6 && dist < best_dist)
            }
        };
        if replace {
            best = Some((area_delta, dist));
        }
    }
    best.map(|(_, dist)| dist)
}

fn stock_from_edges(a: [f64; 3], b: [f64; 3], panel_n: [f64; 3], edges: &[Edge]) -> Option<f64> {
    let mut hits = Vec::new();
    for edge in edges {
        let dir = sub(edge.end, edge.start);
        let len = length(dir);
        if len < 0.15 {
            continue;
        }
        let along = dot(mul(dir, 1.0 / len), panel_n).abs();
        if along < 0.985 {
            continue;
        }
        let touches = near(edge.start, a) || near(edge.end, a) || near(edge.start, b) || near(edge.end, b);
        if touches {
            hits.push(len);
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
    let mid = hits[hits.len() / 2];
    if hits.iter().any(|h| (h - mid).abs() > 0.2) {
        return None;
    }
    Some(mid)
}

/// Die Kantenrichtung zeigt entlang der betragsgrößten Achse nach oben. Dadurch hängt die
/// Seite, an der ein Finger sitzt, nicht davon ab, welchen Endpunkt der Kern als Anfang nennt.
fn canonical(start: [f64; 3], end: [f64; 3]) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let d = sub(end, start);
    let flip = dominant_is_negative(d);
    let (a, b) = if flip { (end, start) } else { (start, end) };
    let x = mul(sub(b, a), 1.0 / length(sub(b, a)));
    (a, b, x)
}

fn dominant_is_negative(d: [f64; 3]) -> bool {
    let mut idx = 0;
    for i in 1..3 {
        if d[i].abs() > d[idx].abs() {
            idx = i;
        }
    }
    d[idx] < 0.0
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    dist(a, b) < 0.02
}

fn vec3(v: &Value) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

fn clean3(v: [f64; 3]) -> [f64; 3] {
    v.map(|c| if c.abs() < 1e-9 { 0.0 } else { c })
}

fn unit(v: [f64; 3]) -> Result<[f64; 3], ()> {
    let n = length(v);
    if n < 1e-12 {
        Err(())
    } else {
        Ok(mul(v, 1.0 / n))
    }
}

pub fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    length(sub(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate() -> (Vec<Edge>, Vec<Face>) {
        let faces = vec![
            Face {
                name: "F1:+z".into(),
                area: 6000.0,
                normal: [0.0, 0.0, 1.0],
                point: [50.0, 30.0, 3.0],
            },
            Face {
                name: "F1:-z".into(),
                area: 6000.0,
                normal: [0.0, 0.0, -1.0],
                point: [50.0, 30.0, 0.0],
            },
            Face {
                name: "F1:-y".into(),
                area: 300.0,
                normal: [0.0, -1.0, 0.0],
                point: [50.0, 0.0, 1.5],
            },
            Face {
                name: "F1:+y".into(),
                area: 300.0,
                normal: [0.0, 1.0, 0.0],
                point: [50.0, 60.0, 1.5],
            },
            Face {
                name: "F1:-x".into(),
                area: 180.0,
                normal: [-1.0, 0.0, 0.0],
                point: [0.0, 30.0, 1.5],
            },
        ];
        let edges = vec![
            Edge {
                index: 1,
                name: Some("F1:+z|F1:-y".into()),
                start: [0.0, 0.0, 3.0],
                end: [100.0, 0.0, 3.0],
                mid: [50.0, 0.0, 3.0],
                length: 100.0,
            },
            Edge {
                index: 3,
                name: Some("F1:+y|F1:+z".into()),
                start: [100.0, 60.0, 3.0],
                end: [0.0, 60.0, 3.0],
                mid: [50.0, 60.0, 3.0],
                length: 100.0,
            },
            Edge {
                index: 9,
                name: Some("F1:-y|F1:-z".into()),
                start: [0.0, 0.0, 0.0],
                end: [100.0, 0.0, 0.0],
                mid: [50.0, 0.0, 0.0],
                length: 100.0,
            },
            Edge {
                index: 8,
                name: Some("F1:-x|F1:-y".into()),
                start: [0.0, 0.0, 0.0],
                end: [0.0, 0.0, 3.0],
                mid: [0.0, 0.0, 1.5],
                length: 3.0,
            },
        ];
        (edges, faces)
    }

    #[test]
    fn top_front_edge_cuts_downward() {
        let (edges, faces) = plate();
        let frame = frame(&edges[0], &faces, &edges).unwrap();
        assert_eq!(frame.direction, "negative");
        assert_eq!(frame.origin, [0.0, 0.0, 3.0]);
        assert_eq!(frame.x, [1.0, 0.0, 0.0]);
        assert_eq!(frame.y, [0.0, 1.0, 0.0]);
        assert!((frame.stock - 3.0).abs() < 1e-6);
        assert!((frame.length - 100.0).abs() < 1e-6);
        assert_eq!(frame.panel_face, "F1:+z");
        assert_eq!(frame.rim_face, "F1:-y");
    }

    #[test]
    fn far_edge_is_flipped_to_positive_x_and_still_enters_the_sheet() {
        let (edges, faces) = plate();
        let frame = frame(&edges[1], &faces, &edges).unwrap();
        assert_eq!(frame.origin, [0.0, 60.0, 3.0]);
        assert_eq!(frame.x, [1.0, 0.0, 0.0]);
        assert_eq!(frame.y, [0.0, -1.0, 0.0]);
        assert_eq!(frame.direction, "positive");
        assert!((frame.stock - 3.0).abs() < 1e-6);
    }

    #[test]
    fn bottom_edge_extrudes_up_into_the_sheet() {
        let (edges, faces) = plate();
        let frame = frame(&edges[2], &faces, &edges).unwrap();
        assert_eq!(frame.direction, "positive");
        assert_eq!(frame.origin, [0.0, 0.0, 0.0]);
        assert_eq!(frame.y, [0.0, 1.0, 0.0]);
        assert!((frame.stock - 3.0).abs() < 1e-6);
    }

    #[test]
    fn thickness_edge_is_refused() {
        let (edges, faces) = plate();
        let err = frame(&edges[3], &faces, &edges).unwrap_err();
        assert!(err.contains("Stirnseite") || err.contains("lang"), "{err}");
    }
}

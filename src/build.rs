//! Baut aus den gerechneten Zinken echte SolidCraft-Features.
//!
//! Zuerst werden alle Kanten gelesen, danach erst Parameter, Skizzen und Schnitte angelegt.
//! Ein Schnitt verändert die Kantenindizes des Körpers; die Ebenen stehen deshalb als feste
//! Weltkoordinaten fest, die Maße als Ausdrücke über `laser_finger`, `laser_kerf`, `laser_fit`,
//! `laser_depth` und `laser_stock`. Wer die Parameter danach ändert, baut die Zinken mit um.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::geom::{self, Edge, Face, Frame};
use crate::joints::{self, Mode, Plan, Spec};

pub trait Runner {
    fn run(&mut self, command: &str, args: Value) -> Result<Value, String>;
    fn progress(&mut self, fraction: f32, message: &str);
    fn cancelled(&mut self) -> bool;
    fn info(&mut self, message: &str);
}

#[derive(Debug)]
pub enum Fail {
    Input { field: String, reason: String },
    Command(String),
    Cancelled,
}

struct Input {
    edges: Vec<Sel>,
    mode: Mode,
    finger_width: Option<f64>,
    kerf: f64,
    thickness: Option<f64>,
    fit: f64,
    fit_percent: f64,
    finger_count: Option<u32>,
    margin_override: Option<f64>,
    inset: bool,
}

struct Sel {
    body: Option<String>,
    index: Option<u32>,
    point: Option<[f64; 3]>,
}

struct Topo {
    edges: Vec<Edge>,
    faces: Vec<Face>,
}

struct Resolved {
    body: String,
    edge: Edge,
    frame: Frame,
}

pub fn make<R: Runner>(r: &mut R, raw: &Value) -> Result<Value, Fail> {
    let input = parse(raw)?;
    validate(&input)?;
    if r.cancelled() {
        return Err(Fail::Cancelled);
    }

    let mut cache: HashMap<String, Topo> = HashMap::new();
    let mut resolved = Vec::new();
    let mut duplicates = 0u32;
    for sel in &input.edges {
        let (body, edge) = resolve(r, &mut cache, sel)?;
        if resolved.iter().any(|e: &Resolved| e.body == body && e.edge.index == edge.index) {
            duplicates += 1;
            continue;
        }
        let frame = {
            let topo = cache.get(&body).ok_or_else(|| command(format!("Topologie von {body} fehlt")))?;
            geom::frame(&edge, &topo.faces, &topo.edges)
                .map_err(|reason| input_err("edges", format!("{body}, Kante {}: {reason}", edge.index)))?
        };
        resolved.push(Resolved { body, edge, frame });
    }
    if resolved.is_empty() {
        return Err(input_err(
            "edges",
            "keine verwendbare Kante. Wähle mindestens eine lange Kante einer Platte.",
        ));
    }

    let stocks: Vec<f64> = resolved.iter().map(|e| e.frame.stock).collect();
    let stock_min = stocks.iter().copied().fold(f64::INFINITY, f64::min);
    let stock_max = stocks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if input.thickness.is_none() && stock_max - stock_min > 0.2 {
        return Err(input_err(
            "material_thickness",
            format!(
                "die Kanten haben unterschiedliche Wandstärken ({stock_min:.2} bis {stock_max:.2} mm). Gib material_thickness an oder verzahne die Platten getrennt."
            ),
        ));
    }
    let stock = round_q((stock_min + stock_max) / 2.0);
    let material = input.thickness.unwrap_or(stock);
    let margin = input.margin_override.unwrap_or(if input.inset { material } else { 0.0 });

    let mut plans = Vec::new();
    for edge in &resolved {
        let spec = Spec {
            length: edge.frame.length,
            thickness: material,
            margin,
            finger_width: input.finger_width,
            finger_count: input.finger_count,
            kerf: input.kerf,
            fit: input.fit,
            fit_percent: input.fit_percent,
            mode: input.mode,
        };
        let plan = joints::plan(&spec).map_err(|reason| {
            input_err(
                "edges",
                format!("{} Kante {}: {reason}", edge.body, edge.edge.index),
            )
        })?;
        plans.push(plan);
    }

    if r.cancelled() {
        return Err(Fail::Cancelled);
    }
    // Namen gelten im ganzen Entwurf. Ein zweiter Aufruf zählt hinter den schon
    // vorhandenen „Box joint …“-Features weiter, sonst lehnt sketch.create ab.
    let first_number = next_joint_number(r)?;
    let width_params = declare_parameters(r, &input, &plans, material, stock)?;

    let mut joints = Vec::new();
    let total = resolved.len();
    for (i, edge) in resolved.iter().enumerate() {
        if r.cancelled() {
            return Err(Fail::Cancelled);
        }
        r.progress(
            i as f32 / total as f32,
            &format!("Kante {} von {total}", i + 1),
        );
        let joint = build_one(
            r,
            first_number + i as u32,
            edge,
            &plans[i],
            &width_params[i],
            input.mode,
            input.fit_percent,
        )?;
        joints.push(joint);
    }
    r.progress(1.0, "Fingerzinken fertig");

    let feature_width = plans[0].feature_width;
    let summary = json!({
        "mode": mode_name(input.mode),
        "edges": joints.len(),
        "duplicates_skipped": duplicates,
        "thickness_mm": material,
        "stock_mm": stock,
        "depth_mm": material,
        "finger_width_mm": plans[0].nominal,
        "adjust_mm": plans[0].adjust,
        "feature_width_mm": feature_width,
        "kerf_mm": input.kerf,
        "fit_mm": input.fit,
        "fit_percent": input.fit_percent,
        "margin_mm": margin,
        "parameters": parameter_names(&width_params),
        "joints": joints,
    });
    r.info(&format!(
        "{} {} an {} Kante(n), Nennbreite {:.3} mm, Ausgleich {:.3} mm",
        joints.iter().map(|j| j["features"].as_u64().unwrap_or(0)).sum::<u64>(),
        mode_name(input.mode),
        joints.len(),
        plans[0].nominal,
        plans[0].adjust
    ));
    Ok(summary)
}

fn declare_parameters<R: Runner>(
    r: &mut R,
    input: &Input,
    plans: &[Plan],
    material: f64,
    stock: f64,
) -> Result<Vec<String>, Fail> {
    let same = plans.windows(2).all(|w| (w[0].nominal - w[1].nominal).abs() < 1e-3);
    let width_params = if same {
        set_param(
            r,
            "laser_finger",
            plans[0].nominal,
            "mm",
            "Nennbreite der Finger vor Kerf und Pressung",
        )?;
        vec!["laser_finger".to_string(); plans.len()]
    } else {
        let mut names = Vec::new();
        for (i, plan) in plans.iter().enumerate() {
            let name = format!("laser_finger_{}", i + 1);
            set_param(
                r,
                &name,
                plan.nominal,
                "mm",
                "Nennbreite dieser Kante vor Kerf und Pressung",
            )?;
            names.push(name);
        }
        names
    };
    set_param(
        r,
        "laser_kerf",
        input.kerf,
        "mm",
        "Laser-Kerf. Finger werden um die Hälfte breiter, Schlitze um die Hälfte enger",
    )?;
    set_param(
        r,
        "laser_fit",
        input.fit,
        "mm",
        "Zusätzliche Pressung in mm. Positiv macht die Verbindung enger",
    )?;
    set_param(
        r,
        "laser_depth",
        material,
        "mm",
        "Wie weit Finger vorstehen und wie tief Schlitze in die Platte reichen",
    )?;
    set_param(
        r,
        "laser_stock",
        stock,
        "mm",
        "Wandstärke, durch die der Schnitt oder der Finger geht",
    )?;
    Ok(width_params)
}

fn parameter_names(width_params: &[String]) -> Vec<String> {
    let mut names: Vec<String> = width_params.to_vec();
    names.sort();
    names.dedup();
    for extra in ["laser_kerf", "laser_fit", "laser_depth", "laser_stock"] {
        names.push(extra.to_string());
    }
    names
}

fn build_one<R: Runner>(
    r: &mut R,
    n: u32,
    edge: &Resolved,
    plan: &Plan,
    width_param: &str,
    mode: Mode,
    fit_percent: f64,
) -> Result<Value, Fail> {
    let sketch_name = format!("Box joint profile {n}");
    let feature_name = format!("Box joint {n}");
    let created = call(
        r,
        "sketch.create",
        json!({
            "name": sketch_name,
            "plane": {
                "origin": edge.frame.origin,
                "x_dir": edge.frame.x,
                "y_dir": edge.frame.y,
            }
        }),
    )?;
    let sketch_id = json_u64(created.get("sketch")).ok_or_else(|| {
        command(format!("sketch.create hat keine Id geliefert: {created}"))
    })?;

    let half = half_expr(mode, width_param, fit_percent);
    let (y0, y1, operation) = match mode {
        Mode::Fingers => ("-laser_depth", "0", "join"),
        Mode::Slots => ("0", "laser_depth", "cut"),
    };
    let mut dof = None;
    for feature in &plan.features {
        let center = format!(
            "(({}) / 2) + ({}) * {}",
            num(edge.frame.length),
            num(feature.coeff),
            width_param
        );
        let reply = call(
            r,
            "sketch.rectangle.two_point",
            json!({
                "p0": [format!("({center}) - ({half})"), y0],
                "p1": [format!("({center}) + ({half})"), y1],
                "connect": false
            }),
        )?;
        if let Some(d) = reply.get("sketch").and_then(|s| json_u64(s.get("dof"))) {
            dof = Some(d);
        }
    }
    let finished = call(r, "sketch.finish", json!({}))?;
    let profiles = json_u64(finished.get("profiles")).unwrap_or(0);
    if profiles != plan.features.len() as u64 {
        return Err(command(format!(
            "{sketch_name} hat {profiles} Profile, erwartet {}",
            plan.features.len()
        )));
    }
    let extruded = call(
        r,
        "solid.extrude",
        json!({
            "sketch": sketch_id,
            "distance": "laser_stock",
            "direction": edge.frame.direction,
            "operation": operation,
            "targets": [edge.body],
            "profiles": "all",
            "name": feature_name
        }),
    )?;
    let mut joint = json!({
        "body": edge.body,
        "edge": edge.edge.index,
        "length_mm": edge.frame.length,
        "stock_mm": edge.frame.stock,
        "segments": plan.segments,
        "features": plan.features.len(),
        "nominal_mm": plan.nominal,
        "feature_width_mm": plan.feature_width,
        "end_gap_mm": plan.end_gap,
        "widened": plan.widened,
        "sketch": sketch_name,
        "sketch_id": sketch_id,
        "feature": feature_name,
        "feature_id": extruded.get("feature").cloned().unwrap_or(Value::Null),
        "direction": edge.frame.direction,
        "operation": operation,
        "panel_face": edge.frame.panel_face,
        "rim_face": edge.frame.rim_face,
    });
    if let Some(d) = dof {
        if d != 0 {
            joint["dof"] = json!(d);
        }
    }
    Ok(joint)
}

/// Halbe Feature-Breite. Das Vorzeichen sitzt vor Kerf, Pressung und Prozent, damit ein Finger
/// wächst und ein Schlitz schrumpft, beide um die Mitte des Abschnitts.
fn half_expr(mode: Mode, width: &str, fit_percent: f64) -> String {
    let sign = match mode {
        Mode::Fingers => "+",
        Mode::Slots => "-",
    };
    if fit_percent.abs() < 1e-12 {
        format!("(({width} {sign} (laser_kerf / 2) {sign} laser_fit) / 2)")
    } else {
        format!(
            "(({width} {sign} (laser_kerf / 2) {sign} laser_fit {sign} ({width} * {})) / 2)",
            num(fit_percent / 100.0)
        )
    }
}

fn resolve<R: Runner>(r: &mut R, cache: &mut HashMap<String, Topo>, sel: &Sel) -> Result<(String, Edge), Fail> {
    if let Some(body) = &sel.body {
        load(r, cache, body)?;
        let edge = pick(&cache[body].edges, sel)?;
        return Ok((body.clone(), edge));
    }
    let Some(point) = sel.point else {
        return Err(input_err(
            "edges",
            "eine Kante braucht einen Körper oder einen Punkt auf der Kante",
        ));
    };
    let bodies = body_names(r)?;
    if bodies.is_empty() {
        return Err(input_err("edges", "im Entwurf ist kein Körper"));
    }
    if bodies.len() > 32 {
        return Err(input_err(
            "edges",
            "ohne Körpername wird nur unter 32 Körpern gesucht",
        ));
    }
    let mut best: Option<(String, Edge, f64)> = None;
    for body in &bodies {
        load(r, cache, body)?;
        for edge in &cache[body].edges {
            let d = geom::dist_point_segment(point, edge.start, edge.end);
            let take = best.as_ref().map(|(_, _, bd)| d < *bd).unwrap_or(true);
            if take {
                best = Some((body.clone(), edge.clone(), d));
            }
        }
    }
    match best {
        Some((body, edge, d)) if d <= 2.0_f64.max(edge.length * 0.05) => Ok((body, edge)),
        Some((_, edge, d)) => Err(input_err(
            "edges",
            format!("der Punkt liegt {d:.2} mm von Kante {} entfernt", edge.index),
        )),
        None => Err(input_err("edges", "keine Kante gefunden")),
    }
}

fn pick(edges: &[Edge], sel: &Sel) -> Result<Edge, Fail> {
    if edges.is_empty() {
        return Err(input_err("edges", "der Körper hat keine Kanten"));
    }
    if let Some(index) = sel.index {
        if let Some(edge) = edges.iter().find(|e| e.index == index) {
            if let Some(point) = sel.point {
                let d = geom::dist_point_segment(point, edge.start, edge.end);
                if d > 2.0_f64.max(edge.length * 0.05) {
                    return nearest(edges, point);
                }
            }
            return Ok(edge.clone());
        }
        if let Some(point) = sel.point {
            return nearest(edges, point);
        }
        return Err(input_err("edges", format!("Kante {index} gibt es an diesem Körper nicht")));
    }
    let Some(point) = sel.point else {
        return Err(input_err("edges", "Kante ohne Index und ohne Punkt"));
    };
    nearest(edges, point)
}

fn nearest(edges: &[Edge], point: [f64; 3]) -> Result<Edge, Fail> {
    let mut best: Option<(&Edge, f64)> = None;
    for edge in edges {
        let d = geom::dist_point_segment(point, edge.start, edge.end);
        if best.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
            best = Some((edge, d));
        }
    }
    match best {
        Some((edge, d)) if d <= 2.0_f64.max(edge.length * 0.05) => Ok(edge.clone()),
        Some((edge, d)) => Err(input_err(
            "edges",
            format!("der Punkt liegt {d:.2} mm von Kante {} entfernt", edge.index),
        )),
        None => Err(input_err("edges", "keine Kante gefunden")),
    }
}

fn load<R: Runner>(r: &mut R, cache: &mut HashMap<String, Topo>, body: &str) -> Result<(), Fail> {
    if cache.contains_key(body) {
        return Ok(());
    }
    let edges_json = call(r, "model.edges", json!({ "body": body }))?;
    let faces_json = call(r, "model.faces", json!({ "body": body }))?;
    let edges = geom::parse_edges(&edges_json).map_err(|e| command(format!("model.edges von {body}: {e}")))?;
    let faces = geom::parse_faces(&faces_json).map_err(|e| command(format!("model.faces von {body}: {e}")))?;
    cache.insert(body.to_string(), Topo { edges, faces });
    Ok(())
}

/// Kleinste freie Nummer für „Box joint N“ und „Box joint profile N“.
fn next_joint_number<R: Runner>(r: &mut R) -> Result<u32, Fail> {
    let doc = call(
        r,
        "document.inspect",
        json!({ "bodies": false, "timeline": "summary", "measure": false }),
    )?;
    let mut highest = 0u32;
    let mut consider = |name: &str| {
        if let Some(n) = joint_number(name) {
            highest = highest.max(n);
        }
    };
    if let Some(sketches) = doc.get("sketches").and_then(Value::as_array) {
        for sketch in sketches {
            if let Some(name) = sketch.get("name").and_then(Value::as_str) {
                consider(name);
            }
        }
    }
    if let Some(timeline) = doc.get("timeline").and_then(Value::as_array) {
        for feature in timeline {
            if let Some(name) = feature.get("name").and_then(Value::as_str) {
                consider(name);
            }
        }
    }
    Ok(highest + 1)
}

/// Nummer am Ende von „Box joint profile 12“ oder „Box joint 12“. Andere Namen zählen nicht.
fn joint_number(name: &str) -> Option<u32> {
    let rest = name
        .strip_prefix("Box joint profile ")
        .or_else(|| name.strip_prefix("Box joint "))?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok().filter(|n| *n > 0)
}

fn body_names<R: Runner>(r: &mut R) -> Result<Vec<String>, Fail> {
    let doc = call(
        r,
        "document.inspect",
        json!({ "bodies": "names", "timeline": false }),
    )?;
    let Some(bodies) = doc.get("bodies").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut names = Vec::new();
    for body in bodies {
        if let Some(name) = body.as_str() {
            names.push(name.to_string());
        } else if let Some(name) = body.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

fn set_param<R: Runner>(r: &mut R, name: &str, value: f64, unit: &str, comment: &str) -> Result<(), Fail> {
    call(
        r,
        "parameters.change",
        json!({
            "name": name,
            "expression": value,
            "unit": unit,
            "comment": comment
        }),
    )?;
    Ok(())
}

fn call<R: Runner>(r: &mut R, command: &str, args: Value) -> Result<Value, Fail> {
    r.run(command, args).map_err(|reason| Fail::Command(format!("{command}: {reason}")))
}

fn parse(raw: &Value) -> Result<Input, Fail> {
    let obj = match raw {
        Value::Object(_) => raw,
        Value::Null => &json!({}),
        _ => return Err(input_err("params", "die Parameter müssen ein JSON-Objekt sein")),
    };
    let edges_v = obj.get("edges").ok_or_else(|| {
        input_err("edges", "mindestens eine Kante ist nötig")
    })?;
    let list: Vec<&Value> = match edges_v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![edges_v],
        Value::Null => {
            return Err(input_err("edges", "mindestens eine Kante ist nötig"));
        }
        _ => return Err(input_err("edges", "edges muss eine Liste von Kanten sein")),
    };
    if list.is_empty() {
        return Err(input_err("edges", "mindestens eine Kante ist nötig"));
    }
    if list.len() > 24 {
        return Err(input_err("edges", "höchstens 24 Kanten in einem Aufruf"));
    }
    let mut edges = Vec::new();
    for (i, item) in list.iter().enumerate() {
        edges.push(parse_sel(item).map_err(|e| match e {
            Fail::Input { reason, .. } => input_err("edges", format!("Kante {}: {reason}", i + 1)),
            other => other,
        })?);
    }

    let mode = match optional_str(obj, &["mode"])?.as_deref() {
        None => Mode::Fingers,
        Some(s) => parse_mode(s)?,
    };
    let finger_width = optional_f64(obj, &["finger_width", "width"], "finger_width")?;
    let finger_width = match finger_width {
        Some(w) if w == 0.0 => None,
        other => other,
    };
    let kerf = optional_f64(obj, &["kerf"], "kerf")?.unwrap_or(0.15);
    let thickness = optional_f64(obj, &["material_thickness", "thickness"], "material_thickness")?;
    let thickness = match thickness {
        Some(t) if t == 0.0 => None,
        other => other,
    };
    let fit = optional_f64(obj, &["fit", "overlap"], "fit")?.unwrap_or(0.0);
    let fit_percent = optional_f64(obj, &["fit_percent", "overlap_percent"], "fit_percent")?.unwrap_or(0.0);
    let finger_count = match optional_f64(obj, &["finger_count", "count"], "finger_count")? {
        None => None,
        Some(n) if (n - n.round()).abs() > 1e-6 => {
            return Err(input_err("finger_count", "die Anzahl muss eine ganze Zahl sein"));
        }
        Some(n) if n < 0.0 => return Err(input_err("finger_count", "die Anzahl darf nicht negativ sein")),
        Some(n) if n == 0.0 => None,
        Some(n) => Some(n.round() as u32),
    };
    let margin_override = optional_f64(obj, &["corner_margin"], "corner_margin")?;
    let inset = match optional_str(obj, &["corners"])?.as_deref() {
        None => true,
        Some(s) => parse_corners(s)?,
    };

    Ok(Input {
        edges,
        mode,
        finger_width,
        kerf,
        thickness,
        fit,
        fit_percent,
        finger_count,
        margin_override,
        inset,
    })
}

fn parse_sel(v: &Value) -> Result<Sel, Fail> {
    if let Some(p) = point_of(v) {
        return Ok(Sel {
            body: None,
            index: None,
            point: Some(p),
        });
    }
    let obj = v.as_object().ok_or_else(|| {
        input_err("edges", "eine Kante ist ein Auswahlobjekt oder ein Punkt [x, y, z]")
    })?;
    if let Some(kind) = obj.get("type").and_then(Value::as_str) {
        if kind != "edge" {
            return Err(input_err("edges", format!("Typ „{kind}“ ist keine Kante")));
        }
    }
    let body = obj
        .get("body")
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let index = obj.get("index").and_then(Value::as_u64).map(|n| n as u32);
    let point = obj.get("point").and_then(point_of).or_else(|| obj.get("at").and_then(point_of));
    if body.is_none() && index.is_none() && point.is_none() {
        return Err(input_err("edges", "Kante ohne Körper, Index oder Punkt"));
    }
    Ok(Sel { body, index, point })
}

fn parse_mode(s: &str) -> Result<Mode, Fail> {
    match s.trim().to_lowercase().as_str() {
        "fingers" | "finger" | "zinken" => Ok(Mode::Fingers),
        "slots" | "slot" | "holes" | "hole" | "schlitze" | "schlitz" | "nuten" | "nut" => Ok(Mode::Slots),
        other => Err(input_err(
            "mode",
            format!("„{other}“ ist weder fingers noch slots"),
        )),
    }
}

fn parse_corners(s: &str) -> Result<bool, Fail> {
    match s.trim().to_lowercase().as_str() {
        "inset" | "einzug" | "clear" | "margin" => Ok(true),
        "flush" | "bündig" | "buendig" | "none" | "full" => Ok(false),
        other => Err(input_err(
            "corners",
            format!("„{other}“ ist weder inset noch flush"),
        )),
    }
}

fn validate(input: &Input) -> Result<(), Fail> {
    if !input.kerf.is_finite() || input.kerf < 0.0 || input.kerf > 5.0 {
        return Err(input_err("kerf", "Kerf muss zwischen 0 und 5 mm liegen"));
    }
    if !input.fit.is_finite() || input.fit < -2.0 || input.fit > 2.0 {
        return Err(input_err("fit", "Pressung muss zwischen -2 und 2 mm liegen"));
    }
    if !input.fit_percent.is_finite() || input.fit_percent < -20.0 || input.fit_percent > 20.0 {
        return Err(input_err(
            "fit_percent",
            "die prozentuale Pressung muss zwischen -20 und 20 liegen",
        ));
    }
    if let Some(w) = input.finger_width {
        if !w.is_finite() || w <= 0.0 || w > 200.0 {
            return Err(input_err(
                "finger_width",
                "Fingerbreite muss zwischen 0 (automatisch) und 200 mm liegen",
            ));
        }
    }
    if let Some(t) = input.thickness {
        if !t.is_finite() || t <= 0.0 || t > 80.0 {
            return Err(input_err(
                "material_thickness",
                "Materialstärke muss zwischen 0 (automatisch) und 80 mm liegen",
            ));
        }
    }
    if let Some(n) = input.finger_count {
        if n == 0 || n > 31 {
            return Err(input_err("finger_count", "Anzahl muss zwischen 1 und 31 liegen"));
        }
    }
    if let Some(m) = input.margin_override {
        if !m.is_finite() || m < 0.0 || m > 100.0 {
            return Err(input_err(
                "corner_margin",
                "Eckeneinzug muss zwischen 0 und 100 mm liegen",
            ));
        }
    }
    Ok(())
}

fn optional_f64(obj: &Value, keys: &[&str], field: &str) -> Result<Option<f64>, Fail> {
    let mut found: Option<f64> = None;
    for key in keys {
        let Some(v) = obj.get(*key) else { continue };
        if v.is_null() {
            continue;
        }
        let n = num_of(v).ok_or_else(|| input_err(field, format!("{key} muss eine Zahl sein")))?;
        if let Some(prev) = found {
            if (prev - n).abs() > 1e-9 {
                return Err(input_err(field, format!("{key} widerspricht einem anderen Wert")));
            }
        }
        found = Some(n);
    }
    Ok(found)
}

fn optional_str(obj: &Value, keys: &[&str]) -> Result<Option<String>, Fail> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if v.is_null() {
                continue;
            }
            let Some(s) = v.as_str() else {
                return Err(input_err(key, format!("{key} muss ein Text sein")));
            };
            return Ok(Some(s.to_string()));
        }
    }
    Ok(None)
}

fn num_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim().trim_end_matches("mm").trim().replace(',', ".");
            if t.is_empty() { None } else { t.parse().ok() }
        }
        _ => None,
    }
}

fn point_of(v: &Value) -> Option<[f64; 3]> {
    if let Some(a) = v.as_array() {
        if a.len() == 3 {
            return Some([num_of(&a[0])?, num_of(&a[1])?, num_of(&a[2])?]);
        }
        return None;
    }
    let obj = v.as_object()?;
    Some([
        num_of(obj.get("x")?)?,
        num_of(obj.get("y")?)?,
        num_of(obj.get("z")?)?,
    ])
}

fn json_u64(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u64))
}

fn num(x: f64) -> String {
    let s = format!("{x:.6}");
    if !s.contains('.') {
        return s;
    }
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

fn round_q(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Fingers => "fingers",
        Mode::Slots => "slots",
    }
}

fn input_err(field: &str, reason: impl Into<String>) -> Fail {
    Fail::Input {
        field: field.to_string(),
        reason: reason.into(),
    }
}

fn command(reason: impl Into<String>) -> Fail {
    Fail::Command(reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rec {
        calls: Vec<(String, Value)>,
        edges: Value,
        faces: Value,
        document: Value,
    }

    impl Runner for Rec {
        fn run(&mut self, command: &str, args: Value) -> Result<Value, String> {
            self.calls.push((command.to_string(), args.clone()));
            match command {
                "model.edges" => Ok(self.edges.clone()),
                "model.faces" => Ok(self.faces.clone()),
                "document.inspect" => Ok(self.document.clone()),
                "parameters.change" => Ok(json!({"name": args["name"], "value": args["expression"]})),
                "sketch.create" => Ok(json!({"sketch": 7})),
                "sketch.rectangle.two_point" => Ok(json!({"sketch": {"dof": 0}})),
                "sketch.finish" => {
                    let mut n = 0usize;
                    for (command, _) in self.calls.iter().rev().skip(1) {
                        if command == "sketch.create" {
                            break;
                        }
                        if command == "sketch.rectangle.two_point" {
                            n += 1;
                        }
                    }
                    Ok(json!({"profiles": n}))
                }
                "solid.extrude" => Ok(json!({"feature": 8, "name": args["name"]})),
                other => Err(format!("unerwartet: {other}")),
            }
        }
        fn progress(&mut self, _: f32, _: &str) {}
        fn cancelled(&mut self) -> bool {
            false
        }
        fn info(&mut self, _: &str) {}
    }

    fn fixture() -> (Value, Value) {
        let edges = json!({
            "body": "Panel",
            "edges": [
                {"index": 1, "name": "F1:+z|F1:-y", "start": [0.0, 0.0, 3.0], "end": [100.0, 0.0, 3.0], "mid": [50.0, 0.0, 3.0], "length": 100.0},
                {"index": 8, "name": "F1:-x|F1:-y", "start": [0.0, 0.0, 0.0], "end": [0.0, 0.0, 3.0], "mid": [0.0, 0.0, 1.5], "length": 3.0},
                {"index": 10, "name": "F1:+x|F1:-y", "start": [100.0, 0.0, 0.0], "end": [100.0, 0.0, 3.0], "mid": [100.0, 0.0, 1.5], "length": 3.0}
            ]
        });
        let faces = json!({
            "body": "Panel",
            "faces": [
                {"name": "F1:+z", "area": 6000.0, "plane_normal": [0.0, 0.0, 1.0], "point": [50.0, 30.0, 3.0]},
                {"name": "F1:-z", "area": 6000.0, "plane_normal": [0.0, 0.0, -1.0], "point": [50.0, 30.0, 0.0]},
                {"name": "F1:-y", "area": 300.0, "plane_normal": [0.0, -1.0, 0.0], "point": [50.0, 0.0, 1.5]},
                {"name": "F1:+y", "area": 300.0, "plane_normal": [0.0, 1.0, 0.0], "point": [50.0, 60.0, 1.5]},
                {"name": "F1:-x", "area": 180.0, "plane_normal": [-1.0, 0.0, 0.0], "point": [0.0, 30.0, 1.5]}
            ]
        });
        (edges, faces)
    }

    struct Stub;
    impl Runner for Stub {
        fn run(&mut self, command: &str, _: Value) -> Result<Value, String> {
            Err(format!("unerwartet: {command}"))
        }
        fn progress(&mut self, _: f32, _: &str) {}
        fn cancelled(&mut self) -> bool {
            false
        }
        fn info(&mut self, _: &str) {}
    }

    #[test]
    fn slots_on_the_front_edge_cut_down_through_the_sheet() {
        let (edges, faces) = fixture();
        let mut rec = Rec {
            calls: Vec::new(),
            edges,
            faces,
            document: json!({ "sketches": [], "timeline": [] }),
        };
        let out = make(
            &mut rec,
            &json!({
                "edges": [{"type": "edge", "body": "Panel", "index": 1, "point": [50.0, 0.0, 3.0]}],
                "mode": "slots",
                "kerf": 0.15
            }),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(out["mode"], "slots");
        assert_eq!(out["edges"], 1);
        assert!((out["finger_width_mm"].as_f64().unwrap() - 7.5).abs() < 1e-9);
        assert!((out["feature_width_mm"].as_f64().unwrap() - 7.425).abs() < 1e-9);
        assert_eq!(out["joints"][0]["features"], 5);
        assert_eq!(out["joints"][0]["operation"], "cut");
        assert_eq!(out["joints"][0]["direction"], "negative");

        let plane = &rec.calls.iter().find(|(c, _)| c == "sketch.create").unwrap().1["plane"];
        assert_eq!(plane["origin"], json!([0.0, 0.0, 3.0]));
        assert_eq!(plane["x_dir"], json!([1.0, 0.0, 0.0]));
        assert_eq!(plane["y_dir"], json!([0.0, 1.0, 0.0]));

        let rects: Vec<_> = rec
            .calls
            .iter()
            .filter(|(c, _)| c == "sketch.rectangle.two_point")
            .collect();
        assert_eq!(rects.len(), 5);
        let p0 = rects[0].1["p0"].as_array().unwrap();
        assert!(p0[0].as_str().unwrap().contains("laser_finger"), "{}", p0[0]);
        assert_eq!(p0[1], "0");
        assert_eq!(rects[0].1["p1"][1], "laser_depth");

        let created = &rec.calls.iter().find(|(c, _)| c == "sketch.create").unwrap().1;
        assert_eq!(created["name"], "Box joint profile 1");
        let extrude = &rec.calls.iter().find(|(c, _)| c == "solid.extrude").unwrap().1;
        assert_eq!(extrude["name"], "Box joint 1");
        assert_eq!(extrude["distance"], "laser_stock");
        assert_eq!(extrude["direction"], "negative");
        assert_eq!(extrude["operation"], "cut");
        assert_eq!(extrude["targets"], json!(["Panel"]));
        assert_eq!(extrude["sketch"], 7);

        let finger = rec
            .calls
            .iter()
            .find(|(c, a)| c == "parameters.change" && a["name"] == "laser_finger")
            .unwrap();
        assert_eq!(finger.1["expression"], json!(7.5));
    }

    #[test]
    fn negative_kerf_never_touches_the_model() {
        let err = make(&mut Stub, &json!({"edges": [{"type": "edge", "body": "Panel", "index": 1}], "kerf": -0.2})).unwrap_err();
        match err {
            Fail::Input { field, .. } => assert_eq!(field, "kerf"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_edges_are_an_input_error() {
        let err = make(&mut Stub, &json!({"mode": "fingers"})).unwrap_err();
        match err {
            Fail::Input { field, .. } => assert_eq!(field, "edges"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_thickness_edge_is_refused_before_a_sketch() {
        let (edges, faces) = fixture();
        let mut rec = Rec {
            calls: Vec::new(),
            edges,
            faces,
            document: json!({ "sketches": [], "timeline": [] }),
        };
        let err = make(
            &mut rec,
            &json!({"edges": [{"type": "edge", "body": "Panel", "index": 8}], "mode": "fingers"}),
        )
        .unwrap_err();
        match err {
            Fail::Input { field, reason } => {
                assert_eq!(field, "edges");
                assert!(reason.contains("Stirnseite") || reason.contains("lang"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        assert!(rec.calls.iter().all(|(c, _)| c == "model.edges" || c == "model.faces"));
    }

    #[test]
    fn a_second_run_continues_the_feature_numbers() {
        assert_eq!(joint_number("Box joint profile 12"), Some(12));
        assert_eq!(joint_number("Box joint 3"), Some(3));
        assert_eq!(joint_number("Box joint profile"), None);
        assert_eq!(joint_number("Panel"), None);

        let (edges, faces) = fixture();
        let mut rec = Rec {
            calls: Vec::new(),
            edges,
            faces,
            document: json!({
                "sketches": [{"name": "Box joint profile 1"}, {"name": "Skizze 4"}],
                "timeline": [{"name": "Panel"}, {"name": "Box joint 1"}, {"name": "Box joint profile 4"}]
            }),
        };
        let out = make(
            &mut rec,
            &json!({
                "edges": [{"type": "edge", "body": "Panel", "index": 1, "point": [50.0, 0.0, 3.0]}],
                "mode": "fingers"
            }),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(out["joints"][0]["sketch"], "Box joint profile 5");
        assert_eq!(out["joints"][0]["feature"], "Box joint 5");
    }
}

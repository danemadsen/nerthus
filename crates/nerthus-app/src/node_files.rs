//! Files node commands write: Synth's Export JSON
//! (`PropertyArea.Synth_ExportColors`).

use std::path::Path;

use nerthus_engine::automation::dotnet_float_string;

/// Writes a Synth's colours (`Synth.Data`, each red, green, blue and
/// position) as `XRGBA`s: a `.csv` file gets a `position,red,green,blue`
/// line each (`XRGBA.ToString()`), anything else a JSON array (indented,
/// `X`, `R`, `G`, `B` and an `A` of 0). The extension decides between them.
pub fn write_synth_colours(path: &Path, colours: &[[f32; 4]]) -> std::io::Result<()> {
    let is_csv = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("csv"));
    let text = if is_csv { synth_lines(colours) } else { synth_json(colours) };
    std::fs::write(path, text)
}

/// `StringBuilder.AppendLine(new XRGBA(colour).ToString())` for each.
fn synth_lines(colours: &[[f32; 4]]) -> String {
    let mut text = String::new();
    for [red, green, blue, position] in colours {
        let line = format!("{},{},{},{}", dotnet_float_string(*position), dotnet_float_string(*red), dotnet_float_string(*green), dotnet_float_string(*blue));
        text.push_str(&line);
        text.push_str("\r\n");
    }
    text
}

/// `JsonConvert.SerializeObject(colours.Select(c => new XRGBA(c)),
/// Formatting.Indented)`.
fn synth_json(colours: &[[f32; 4]]) -> String {
    if colours.is_empty() {
        return "[]".to_string();
    }
    let mut text = String::from("[");
    for (index, [red, green, blue, position]) in colours.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str("\r\n  {");
        let members = [("X", *position), ("R", *red), ("G", *green), ("B", *blue), ("A", 0.0)];
        for (member_index, (name, value)) in members.iter().enumerate() {
            if member_index > 0 {
                text.push(',');
            }
            text.push_str(&format!("\r\n    \"{name}\": {}", newtonsoft_float(*value)));
        }
        text.push_str("\r\n  }");
    }
    text.push_str("\r\n]");
    text
}

/// `JsonConvert.ToString(float)`: the shortest round-trip text with `.0`
/// added when it has neither a point nor an exponent; NaN and infinities
/// quoted.
fn newtonsoft_float(value: f32) -> String {
    let text = dotnet_float_string(value);
    if value.is_nan() || value.is_infinite() {
        return format!("\"{text}\"");
    }
    if text.contains('.') || text.contains('E') {
        return text;
    }
    format!("{text}.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_colours_as_lines_and_json() {
        let colours = [[0.5, 0.25, 1.0, 0.0], [0.000001, 0.0, 0.75, 0.5]];
        assert_eq!(synth_lines(&colours), "0,0.5,0.25,1\r\n0.5,1E-06,0,0.75\r\n");
        let json = synth_json(&colours[..1]);
        assert_eq!(json, "[\r\n  {\r\n    \"X\": 0.0,\r\n    \"R\": 0.5,\r\n    \"G\": 0.25,\r\n    \"B\": 1.0,\r\n    \"A\": 0.0\r\n  }\r\n]");
    }
}

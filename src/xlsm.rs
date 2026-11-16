//! Excel (.xlsm/.xlsx/.xls/.xlsb) workbook reader.
//!
//! The UI's XLSM importer expects the same shape that SheetJS produced in
//! Electron: `{ SheetNames: [...], Sheets: { name: [[cell, ...], ...] } }`
//! with `header: 1`, `defval: ''`, `raw: true` and dates as `Date` objects.
//! JSON cannot carry a JS Date, so date cells are emitted as ISO `YYYY-MM-DD`
//! strings; the UI's xlsmDate() already accepts that form.

use calamine::{Data, ExcelDateTime, Reader};
use serde_json::{json, Map, Value};
use std::io::Cursor;

pub fn read_workbook(bytes: Vec<u8>) -> Result<Value, String> {
    let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e| format!("Unable to read workbook: {e}"))?;

    let sheet_names = workbook.sheet_names().to_owned();
    let mut sheets = Map::new();

    for name in &sheet_names {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| format!("{name}: {e}"))?;
        sheets.insert(name.clone(), Value::Array(range_to_json(&range)));
    }

    Ok(json!({
        "SheetNames": sheet_names,
        "Sheets": sheets
    }))
}

fn range_to_json(range: &calamine::Range<Data>) -> Vec<Value> {
    let Some((start_row, start_col)) = range.start() else {
        return Vec::new();
    };
    let (height, width) = range.get_size();
    let total_width = start_col as usize + width;

    let mut rows = Vec::with_capacity(start_row as usize + height);
    for _ in 0..start_row {
        rows.push(Value::Array(vec![json!(""); total_width]));
    }

    for row in range.rows() {
        let mut cells = Vec::with_capacity(total_width);
        for _ in 0..start_col {
            cells.push(json!(""));
        }
        cells.extend(row.iter().map(cell_to_json));
        while cells.len() < total_width {
            cells.push(json!(""));
        }
        rows.push(Value::Array(cells));
    }

    rows
}

fn cell_to_json(cell: &Data) -> Value {
    match cell {
        Data::Empty => json!(""),
        Data::String(value) => json!(value),
        Data::Int(value) => json!(value),
        Data::Float(value) if value.is_finite() => json!(value),
        Data::Float(_) => json!(""),
        Data::Bool(value) => json!(value),
        Data::DateTime(value) => json!(date_string(value)),
        Data::DateTimeIso(value) => json!(value),
        Data::DurationIso(value) => json!(value),
        Data::Error(_) => json!(""),
    }
}

fn date_string(value: &ExcelDateTime) -> String {
    let (year, month, day, _, _, _, _) = value.to_ymd_hms_milli();
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a small synthetic ledger workbook in memory (no real lab data
    /// is kept in the repository).
    fn sample_workbook() -> Vec<u8> {
        use rust_xlsxwriter::{ExcelDateTime as XlDate, Format, Workbook};
        let mut book = Workbook::new();
        let date = Format::new().set_num_format("dd/mm/yyyy");
        let log = book.add_worksheet().set_name("Log").unwrap();
        for (col, title) in ["item", "lot", "quantity", "exp"].iter().enumerate() {
            log.write_string(0, u16::try_from(col).unwrap(), *title).unwrap();
        }
        log.write_string(1, 0, "Sample reagent (box)").unwrap();
        log.write_string(1, 1, "LOT-001").unwrap();
        log.write_number(1, 2, 4).unwrap();
        log.write_datetime_with_format(1, 3, XlDate::from_ymd(2025, 10, 13).unwrap(), &date)
            .unwrap();
        // A sheet whose data starts at C3 checks the leading padding.
        let other = book.add_worksheet().set_name("Offset").unwrap();
        other.write_string(2, 2, "catalog").unwrap();
        book.save_to_buffer().unwrap()
    }

    #[test]
    fn reads_a_workbook() {
        let workbook = read_workbook(sample_workbook()).expect("workbook should parse");
        assert_eq!(workbook["SheetNames"], json!(["Log", "Offset"]));
        let log = &workbook["Sheets"]["Log"];
        assert_eq!(log[0], json!(["item", "lot", "quantity", "exp"]));
        assert_eq!(log[1], json!(["Sample reagent (box)", "LOT-001", 4.0, "2025-10-13"]));
        let offset = &workbook["Sheets"]["Offset"];
        assert_eq!(offset[0], json!(["", "", ""]));
        assert_eq!(offset[2], json!(["", "", "catalog"]));
    }

    #[test]
    fn rejects_non_workbooks() {
        assert!(read_workbook(b"not a workbook".to_vec()).is_err());
    }

    #[test]
    fn date_cells_are_iso_dates() {
        let value = ExcelDateTime::new(45943.0, calamine::ExcelDateTimeType::DateTime, false);
        assert_eq!(date_string(&value), "2025-10-13");
    }
}

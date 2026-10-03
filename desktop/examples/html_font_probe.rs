//! Research helper: verify GPUI font APIs needed by a native HTML adapter.
//! No windows, mail database, HTTP client or production service is opened.
use gpui_kit::*;
use serde_json::json;

fn main() {
    let output = std::env::args().nth(1).expect("Pass an output JSON path");
    gpui_kit::application().run(move |cx| {
        let system = WindowTextSystem::new(cx.text_system().clone());
        let mut measurements = vec![];
        for family in ["Segoe UI", "Microsoft YaHei UI"] {
            for text in ["Alpha 中文 Beta", "服务账单 30.00", "👩‍🚀 e\u{301} العربية"] {
                for weight in [400., 700.] {
                    let font = Font { family: family.into(), weight: FontWeight(weight), ..Default::default() };
                    let id = system.resolve_font(&font);
                    let line = system.shape_line(text.into(), px(16.), &[TextRun {
                        len: text.len(), font, color: rgb(0x202724).into(),
                        background_color: None, underline: None, strikethrough: None,
                    }], None);
                    measurements.push(json!({"font":family,"weight":weight,"text":text,
                        "width":f32::from(line.width),"ascent":f32::from(system.ascent(id,px(16.))),
                        "descent":f32::from(system.descent(id,px(16.))),"xHeight":f32::from(system.x_height(id,px(16.))),
                        "endIndex":line.closest_index_for_x(line.width)}));
                }
            }
        }
        std::fs::write(&output, serde_json::to_vec_pretty(&measurements).unwrap()).expect("Write font measurements");
        cx.quit();
    });
}

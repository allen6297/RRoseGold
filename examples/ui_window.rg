# Live UI demo. Opens an eframe window — not run by cargo test.
#   rosegold run examples/ui_window.rg
#   cargo run --offline -- run examples/ui_window.rg
import ui;

fn main(): Int {
    ui.theme({
        "bg": "#1b1b1b",
        "text": "#f2e6dc",
        "accent": "#c45c26",
        "font_size": 14,
    });
    ui.window("Demo") {
        ui.column {
            ui.text("Hello");
            ui.button("OK") { ui.alert("hi"); }
                .padding(8)
                .color("#c45c26")
                .width(120);
        };
    };
    return ui.run();
}

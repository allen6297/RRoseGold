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
    var name = "hi";
    var on = false;
    var vol = 0.5;
    ui.window("Demo") {
        ui.column {
            ui.row {
                ui.text("Name");
                name = ui.field(name);
            };
            on = ui.checkbox("Loud", on);
            vol = ui.slider(vol, 0.0, 1.0);
            ui.text(name);
            ui.separator();
            ui.spacer();
            ui.button("OK") { ui.alert("hi"); }
                .padding(8)
                .color("#c45c26")
                .width(120);
            ui.button("Quit") { ui.quit(); };
        };
    };
    return ui.run();
}

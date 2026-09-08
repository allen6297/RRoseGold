# Live UI demo. Opens an eframe window — not run by cargo test.
# Use the workspace binary (PATH `rosegold` may be an older cargo-install):
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
    var choice = "a";
    var path = "";
    ui.window("Demo") {
        ui.scroll {
            ui.row {
                ui.text("Name");
                name = ui.field(name);
            };
            on = ui.checkbox("Loud", on);
            vol = ui.slider(vol, 0.0, 1.0);
            choice = ui.select(["a", "b", "c"], choice);
            ui.progress(vol);
            ui.scroll {
                for i in 1..100 {
                    ui.text(f"line {i}");
                }
            }.width(200).height(180);
            ui.text(name);
            ui.separator();
            ui.spacer();
            ui.button("Open") {
                match ui.open() {
                    Some(p) { path = p; }
                    None {}
                }
            };
            ui.button("Save") {
                match ui.save() {
                    Some(p) { path = p; }
                    None {}
                }
            };
            ui.text(path);
            ui.button("OK") { ui.alert("hi"); }
                .padding(8)
                .color("#c45c26")
                .width(120);
            ui.button("Quit") { ui.quit(); };
        };
    };
    return ui.run();
}

# Headless UI sample. Do not call ui.run() here — that opens a window.
# Live window: rosegold run examples/ui_window.rg
import ui;

fn main(): Int {
    ui.theme({
        "bg": "#1b1b1b",
        "text": "#f2e6dc",
        "accent": "#c45c26",
        "font_size": 14,
    });
    ui.alert("hi");
    var ok = ui.button("OK") { ui.alert("clicked"); }
        .padding(8)
        .color("#c45c26")
        .width(120);
    var name = "hi";
    var on = false;
    var vol = 0.5;
    ui.column {
        ui.text("Hello");
        ui.row {
            ui.text("Name");
            name = ui.field(name);
        };
        on = ui.checkbox("Loud", on);
        vol = ui.slider(vol, 0.0, 1.0);
        ui.separator();
        ui.spacer();
    };
    ui.invalidate();
    print(ok.style["padding"]);
    print(name);
    print(on);
    print(vol);
    return 0;
}

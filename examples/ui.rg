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
    ui.column {
        ui.text("Hello");
    };
    print(ok.style["padding"]);
    return 0;
}

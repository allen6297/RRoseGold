# Public UI API. Window / run loop / paint live in `__ui.*`.
# Theme maps and v1 modifiers stay here (handles are `Widget` instances).

pub mod ui {
    ## Built-in Widgets, containers, and primitives.
    pub class Widget {
        var kind: String = "";
        var label: String = "";
        var style: Map = {};

        fn padding(self, n: Int): Widget {
            self.style["padding"] = n;
            return self;
        }

        fn color(self, c: String): Widget {
            self.style["color"] = c;
            return self;
        }

        fn bg(self, c: String): Widget {
            self.style["bg"] = c;
            return self;
        }

        fn width(self, n: Int): Widget {
            self.style["width"] = n;
            return self;
        }

        fn height(self, n: Int): Widget {
            self.style["height"] = n;
            return self;
        }

        fn font_size(self, n: Int): Widget {
            self.style["font_size"] = n;
            return self;
        }

        fn disabled(self, on: Bool): Widget {
            self.style["disabled"] = on;
            return self;
        }
    }

    pub fn theme(m: Map): Map {
        __ui.theme(m);
        return m;
    }

    pub fn alert(msg: String) {
        __ui.alert(msg);
    }

    pub fn open(): Option {
        return __ui.open();
    }

    pub fn save(): Option {
        return __ui.save();
    }

    pub fn window(title: String, body: Fn): Widget {
        var h = Widget { kind: "window", label: title };
        __ui.window(h, body);
        return h;
    }

    pub fn column(body: Fn): Widget {
        var h = Widget { kind: "column" };
        __ui.widget(h);
        __ui.begin(h);
        body();
        __ui.end();
        return h;
    }

    pub fn row(body: Fn): Widget {
        var h = Widget { kind: "row" };
        __ui.widget(h);
        __ui.begin(h);
        body();
        __ui.end();
        return h;
    }

    pub fn scroll(body: Fn): Widget {
        var h = Widget { kind: "scroll" };
        __ui.widget(h);
        __ui.begin(h);
        body();
        __ui.end();
        return h;
    }

    pub fn text(s: String): Widget {
        var h = Widget { kind: "text", label: s };
        __ui.widget(h);
        return h;
    }

    pub fn field(s: String): String {
        var h = Widget { kind: "field", label: s };
        return __ui.field(h);
    }

    pub fn button(label: String, action: Fn): Widget {
        var h = Widget { kind: "button", label: label };
        __ui.widget(h);
        __ui.bind(h, action);
        return h;
    }

    pub fn checkbox(label: String, on: Bool): Bool {
        var h = Widget { kind: "checkbox", label: label };
        h.style["on"] = on;
        return __ui.checkbox(h);
    }

    pub fn slider(value: Float, min: Float, max: Float): Float {
        var h = Widget { kind: "slider" };
        h.style["value"] = value;
        h.style["min"] = min;
        h.style["max"] = max;
        return __ui.slider(h);
    }

    pub fn select(options: Array, current: String): String {
        var h = Widget { kind: "select", label: current };
        h.style["options"] = options;
        return __ui.select(h);
    }

    pub fn progress(t: Float): Widget {
        var v = t;
        if v < 0.0 {
            v = 0.0;
        }
        if v > 1.0 {
            v = 1.0;
        }
        var h = Widget { kind: "progress" };
        h.style["value"] = v;
        __ui.widget(h);
        return h;
    }

    pub fn separator(): Widget {
        var h = Widget { kind: "separator" };
        __ui.widget(h);
        return h;
    }

    pub fn spacer(): Widget {
        var h = Widget { kind: "spacer" };
        __ui.widget(h);
        return h;
    }

    pub fn quit() {
        __ui.quit();
    }

    pub fn invalidate() {
        __ui.invalidate();
    }

    pub fn run(): Int {
        return __ui.run();
    }
}

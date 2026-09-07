# Public UI API. Window / run loop / paint live in `__ui.*`.
# Theme maps and v1 modifiers stay here (handles are `Widget` instances).

pub mod ui {
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

    pub fn text(s: String): Widget {
        var h = Widget { kind: "text", label: s };
        __ui.widget(h);
        return h;
    }

    pub fn button(label: String, action: Fn): Widget {
        var h = Widget { kind: "button", label: label };
        __ui.widget(h);
        __ui.bind(h, action);
        return h;
    }

    pub fn run(): Int {
        return __ui.run();
    }
}

# `rosegold run examples/signals.rg`
signal collected(amount: Int);

fn log_coin(amount: Int) {
    print(f"log {amount}");
}

fn total_coin(amount: Int) {
    print(f"total {amount}");
}

fn main(): Int {
    collected.connect(log_coin);
    collected.connect(total_coin);
    collected.emit(5);
    return 0;
}

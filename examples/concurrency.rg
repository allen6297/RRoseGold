# `rosegold run examples/concurrency.rg`
async fn add(a: Int, b: Int): Int {
    return a + b;
}

fn bump(xs: Array, lock: Mutex) {
    lock.lock();
    xs.push(1);
    lock.unlock();
}

fn ping(ch: Channel, n: Int) {
    ch.send(n);
}

fn hang(ch: Channel) {
    ch.recv();
}

fn main(): Int {
    print(await add(2, 3));

    var xs = [];
    var lock = Mutex();
    var t = spawn bump(xs, lock);
    bump(xs, lock);
    await t;
    print(xs.len());

    var ch = Channel();
    var p = spawn ping(ch, 7);
    print(ch.recv());
    await p;
    ch.close();
    print(ch.recv());
    print(ch.recv_timeout(0.0).is_none());

    var stuck = Channel();
    var h = spawn hang(stuck);
    print(h.wait(0.05).is_none());
    stuck.close();
    await h;
    return 0;
}

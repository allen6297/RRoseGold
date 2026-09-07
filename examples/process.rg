# `rosegold run examples/process.rg hello`
import process;

fn main(): Int {
    var args = process.argv();
    print(args.len());
    if args.len() > 1 {
        print(args[1]);
    }
    match process.env("PATH") {
        Some(_) { print("path"); }
        None { print("no-path"); }
    }
    return 0;
}

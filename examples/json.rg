# `rosegold run examples/json.rg`
import json;

fn main(): Int {
    var encoded = json.stringify([1, "two", none]).unwrap();
    print(encoded);

    var data = json.parse("{\"n\": 1, \"ok\": true, \"xs\": [1, 2]}").unwrap();
    print(data["n"]);
    print(data["ok"]);
    print(len(data["xs"]));
    
    var round = json.parse(json.stringify(data).unwrap()).unwrap();
    print(round["n"]);
    print(json.parse("not-json").is_err());
    return 0;
}

# Scene node types. `import strata.Sprite;` (or Node / Empty / …) before `extends Sprite`.
# Play does not preload these; the import loads this module.

pub mod strata {
## Transform only. Parent other nodes. Same kind as Empty.
## Override `on_create` / `on_update` / `on_destroy` / `on_enter` / `on_exit` in an `@node` class.
pub class Node {
    var name: String = "";
    var x: Float = 0.0;
    var y: Float = 0.0;
    var z: Float = 0.0;

    ## Called once when Play starts (engine `on_ready`). Use `self.x` / `self.y` / `self.z`.
    fn on_create(self) {
        pass;
    }

    ## Called every Play frame. `dt` is seconds.
    fn on_update(self, dt: Float) {
        pass;
    }

    ## Called once when this node is removed.
    fn on_destroy(self) {
        pass;
    }

    ## Fired when this node's AABB starts overlapping `other` (that entity's name).
    fn on_enter(self, other: Str) {
        pass;
    }

    ## Fired when this node's AABB stops overlapping `other`.
    fn on_exit(self, other: Str) {
        pass;
    }
}

## Transform only. Parent other nodes.
pub class Empty extends Node {
}

## Textured quad. Draw and collide.
pub class Sprite extends Node {
}

## Paint a grid. Solid cells are walls.
pub class Tilemap extends Node {
}

## Play follow and view frustum.
pub class Camera extends Node {
}

## 3D primitive in the editor viewport.
pub class Mesh extends Node {
}

## 3D light in the editor viewport.
pub class Light extends Node {
}
}

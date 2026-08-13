import std.{*};
import std.dispatch.{*};
import std.Generic.{*};

pragma no-patterson-condition;
pragma no-bounded-variable-condition;

forall a . class a:CloneLike {
  function clone(x : a) -> a;
}

instance uint256:CloneLike {
  function clone(x : uint256) -> uint256 { return x; }
}

contract DeriveClass {
  #[derive(Eq, Ord)]
  data Color = Red | Green | Blue;

  #[derive(Eq, Ord)]
  data Point = Point(uint256, uint256);

  #[derive(CloneLike)]
  data Box = Box(uint256);

  constructor() {}

  // #[() -> 1]
  public function eqRedRed() -> uint256 {
    match Eq.eq(Color.Red, Color.Red) {
    | true => return uint256(1);
    | false => return uint256(0);
    }
  }

  // #[() -> 0]
  public function eqRedBlue() -> uint256 {
    match Eq.eq(Color.Red, Color.Blue) {
    | true => return uint256(1);
    | false => return uint256(0);
    }
  }

  // #[() -> 1]
  public function gtGreenRed() -> uint256 {
    match Ord.gt(Color.Green, Color.Red) {
    | true => return uint256(1);
    | false => return uint256(0);
    }
  }

  // #[() -> 0]
  public function gtRedGreen() -> uint256 {
    match Ord.gt(Color.Red, Color.Green) {
    | true => return uint256(1);
    | false => return uint256(0);
    }
  }

  // #[() -> 1]
  public function eqPointSame() -> uint256 {
    match Eq.eq(Point(uint256(1), uint256(2)), Point(uint256(1), uint256(2))) {
    | true => return uint256(1);
    | false => return uint256(0);
    }
  }

  // #[() -> 1]
  public function gtPointLex() -> uint256 {
    match Ord.gt(Point(uint256(1), uint256(100)), Point(uint256(1), uint256(50))) {
    | true => return uint256(1);
    | false => return uint256(0);
    }
  }

  // #[(42) -> 42]
  // #[(3735928559) -> 3735928559]
  public function clonePayload(x : uint256) -> uint256 {
    match CloneLike.clone(Box(x)) {
    | Box(value) => return value;
    }
  }
}

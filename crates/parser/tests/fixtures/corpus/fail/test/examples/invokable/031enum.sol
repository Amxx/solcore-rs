function addW(x: Word, y: Word) returns (Word) {
   let res : Word;
   assembly {
       res := add(x, y)
    }
    return res;
}

trait Enum<a> {
    function fromEnum(x: a) returns (Word) ;
  }

  enum Color { R, G, B }

impl Enum<Color> {
  function fromEnum(c: Color) returns (Word) {
    match (c) {
case R {
return 1;
}
case Color.G {
return 2;
}
case Color.B {
return 3;
}
}
  }
}

enum Bool { False, True }

instance Bool : Enum {
  function fromEnum(b) {
      match b {
      | False => return 0;
      | Bool.True => return 1;
      };
  }
}
data FromEnumToken(a) = FromEnumToken

class self : Invokable(args, ret) {
    function invoke (s:self,  a:args) -> ret;
}

instance (a:Enum) => FromEnumToken(a) : Invokable(a,Word) {
   function invoke(fet : FromEnumToken(a), arg) -> Word {
     return fromEnum(arg);
   }
}
contract RGB {
  public function main() {
  /*
  let x = fromEnum(Color.B);
  let y = fromEnum(Bool.True);
  */

  let fetC = FromEnumToken;
  let fetB = FromEnumToken;
  let x = invoke(fetC, Color.B);
  let y = invoke(fetB,Bool.True);
  return addW(x,y);
  }
}

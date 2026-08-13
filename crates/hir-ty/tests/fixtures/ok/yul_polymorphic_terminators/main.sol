forall a . function viaStop() -> a {
  assembly {
    stop()
  }
}

forall a . function viaInvalid() -> a {
  assembly {
    invalid()
  }
}

forall a . function viaSelfdestruct(beneficiary : word) -> a {
  assembly {
    selfdestruct(beneficiary)
  }
}

forall a . function viaRevert() -> a {
  assembly {
    revert(0, 0)
  }
}

function useWord(value : word) -> () {}

contract Terminators {
  public function main() -> () {
    useWord(viaStop());
    useWord(viaInvalid());
    useWord(viaSelfdestruct(0));
    useWord(viaRevert());
  }
}

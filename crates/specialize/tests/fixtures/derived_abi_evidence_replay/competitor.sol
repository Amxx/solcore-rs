import abi.{*};
import reexport.{Leaf};

export { keepCompetitorReachable };

// This orphan is reachable from the entry module but is not visible in the
// module that defines Box. A derived wrapper must replay definition-side
// evidence rather than scanning every reachable environment.
instance Leaf:ABIAttribs {
  function headSize(ty:Proxy(Leaf)) -> word { return 64; }
  function isStatic(ty:Proxy(Leaf)) -> bool { return true; }
}

function keepCompetitorReachable() -> word { return 0; }

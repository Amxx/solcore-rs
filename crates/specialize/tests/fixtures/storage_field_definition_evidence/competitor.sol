import api.{storage, uint256, CanStore};

export { keepCompetitorReachable };

instance storage(uint256):CanStore(uint256) {
  function store(r:storage(uint256), v:uint256) -> () {
    assembly { sstore(99, 99) }
  }

  function load(r:storage(uint256)) -> uint256 {
    let result:word;
    assembly { result := sload(99) }
    return uint256(99);
  }
}

function keepCompetitorReachable() -> word { return 0; }

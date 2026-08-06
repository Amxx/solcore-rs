export { Marker, Box, Phantom };

forall a .
class a:Marker {
  function mark(x: a) -> word;
}

instance word:Marker {
  function mark(x: word) -> word {
    return x;
  }
}

#[derive(Marker)]
data Box(a) = Box(a);

#[derive(Marker)]
data Phantom(a) = Phantom(word);

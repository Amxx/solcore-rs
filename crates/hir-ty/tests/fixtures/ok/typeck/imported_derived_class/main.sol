import lib.{Marker, Box, Phantom};

// The derived instance is declared in an imported module and recursively
// discharges the class constraint for every declared type parameter.
function markBox(x: Box(word)) -> word {
  return Marker.mark(x);
}

forall a . a:Marker =>
function markPhantom(x: Phantom(a)) -> word {
  return Marker.mark(x);
}

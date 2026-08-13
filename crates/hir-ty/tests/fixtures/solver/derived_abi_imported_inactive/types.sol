import generic.{Generic};

export { Box(*) };

// Generic is visible here, but the ABIDeriving marker deliberately is not.
data Box(a) = Box(a);

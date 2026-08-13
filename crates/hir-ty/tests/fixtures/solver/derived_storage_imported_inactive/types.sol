import generic.{Generic};

export { Box(*) };

// Generic is visible here, but StorageDeriving deliberately is not.
data Box(a) = Box(a);

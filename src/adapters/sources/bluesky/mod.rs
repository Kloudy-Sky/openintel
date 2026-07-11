// Transient: the parser lands before the HTTP source that calls it (next task
// removes this file-scoped allow when `BlueskySource` wires everything up).
#![allow(dead_code)]

mod response;

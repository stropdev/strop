//! The stale-result race, explored exhaustively over the shipped store
//! (0099 §5.1).
//!
//! A build reads a frozen document; its result arrives on the event loop
//! after whatever edits, restarts and closes were handled in between. This
//! test runs **every** sequence of those events up to a fixed length
//! against the real [`SymbolStore`] and asks, after each step, the one
//! question that matters: is every list a lookup serves exactly the list of
//! the current text, from the producer asked?
//!
//! The explorer is calibrated like the proofs are: two wrong acceptance
//! rules, written here, must each be caught. If it could not find the bug
//! it exists for, it would prove nothing.

use super::store::{Build, BuildId, Current, Producer, SymbolStore};
use strop_core::id::{BufferRevision, DocumentId, LineIndex};
use strop_core::worker::WorkerId;
use strop_lsp::{ProtoSymbol, ServerColumn, ServerId, ServerLocation, ServerPosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Edit,
    BuildOutline,
    BuildServer,
    DeliverOldest,
    DeliverNewest,
    RestartServer,
    Reopen,
    Supersede,
}

const STEPS: [Step; 8] = [
    Step::Edit,
    Step::BuildOutline,
    Step::BuildServer,
    Step::DeliverOldest,
    Step::DeliverNewest,
    Step::RestartServer,
    Step::Reopen,
    Step::Supersede,
];

fn document(generation: u32) -> DocumentId {
    let fields = [("slot", 0u32), ("generation", generation)];
    serde::Deserialize::deserialize(serde::de::value::MapDeserializer::<
        _,
        serde::de::value::Error,
    >::new(fields.into_iter()))
    .expect("a document id")
}

/// The list a correct build returns for this text: its one row names the
/// document incarnation, the revision and the producer it was read from.
fn list(document: DocumentId, revision: u64, producer: Producer) -> Vec<ProtoSymbol> {
    vec![ProtoSymbol {
        name: format!("{}@{revision} by {producer:?}", document.generation()),
        container: String::new(),
        kind: "Function".into(),
        location: ServerLocation {
            doc: strop_workspace::ResourceLocation::local("/w/a.rs".into()),
            position: ServerPosition {
                line: LineIndex::new(0),
                column: ServerColumn::new(0),
            },
        },
    }]
}

/// How a result is handed to the store. The shipped rule is `honest`.
type Accept = fn(&mut SymbolStore, &World, BuildId, Vec<ProtoSymbol>);

struct World {
    generation: u32,
    revision: u64,
    server: u64,
    next_build: u64,
    /// Results on their way to the event loop, oldest first.
    in_flight: Vec<(BuildId, Vec<ProtoSymbol>)>,
}

impl World {
    fn document(&self) -> DocumentId {
        document(self.generation)
    }
    fn current(&self) -> Current {
        Current {
            revision: BufferRevision::new(self.revision),
            server: Some(ServerId::new(self.server)),
        }
    }
}

fn honest(store: &mut SymbolStore, world: &World, id: BuildId, symbols: Vec<ProtoSymbol>) {
    store.deliver(id, Some(world.current()), Some(symbols));
}

/// Wrong: the result is filed under the revision current when it arrives.
fn stamped_at_delivery(
    store: &mut SymbolStore,
    world: &World,
    id: BuildId,
    symbols: Vec<ProtoSymbol>,
) {
    if let Some(build) = store.building().copied().filter(|build| build.id == id) {
        store.abandon();
        store.begin(Build {
            revision: BufferRevision::new(world.revision),
            document: world.document(),
            ..build
        });
    }
    store.deliver(id, Some(world.current()), Some(symbols));
}

/// Wrong: whoever computed the list, it is filed as the server's.
fn filed_as_the_server(
    store: &mut SymbolStore,
    world: &World,
    id: BuildId,
    symbols: Vec<ProtoSymbol>,
) {
    if let Some(build) = store.building().copied().filter(|build| build.id == id) {
        store.abandon();
        store.begin(Build {
            producer: Producer::Server(ServerId::new(world.server)),
            ..build
        });
    }
    store.deliver(id, Some(world.current()), Some(symbols));
}

/// Run one sequence; `Err` names the first step after which a lookup
/// served something other than the current text's list.
fn run(sequence: &[Step], accept: Accept) -> Result<(), String> {
    let mut store = SymbolStore::default();
    let mut world = World {
        generation: 0,
        revision: 0,
        server: 0,
        next_build: 1,
        in_flight: Vec::new(),
    };
    for (at, step) in sequence.iter().enumerate() {
        match step {
            Step::Edit => world.revision += 1,
            Step::BuildOutline | Step::BuildServer => {
                if store.building().is_none() {
                    let producer = if *step == Step::BuildOutline {
                        Producer::Outline
                    } else {
                        Producer::Server(ServerId::new(world.server))
                    };
                    let id = BuildId::Outline(WorkerId::new(world.next_build));
                    world.next_build += 1;
                    store.begin(Build {
                        id,
                        document: world.document(),
                        revision: BufferRevision::new(world.revision),
                        producer,
                        open: false,
                    });
                    // the build reads the text as it is now
                    world
                        .in_flight
                        .push((id, list(world.document(), world.revision, producer)));
                }
            }
            Step::DeliverOldest | Step::DeliverNewest => {
                if !world.in_flight.is_empty() {
                    let index = if *step == Step::DeliverOldest {
                        0
                    } else {
                        world.in_flight.len() - 1
                    };
                    let (id, symbols) = world.in_flight.remove(index);
                    accept(&mut store, &world, id, symbols);
                }
            }
            Step::RestartServer => {
                store.retire_server(ServerId::new(world.server));
                world.server += 1;
            }
            Step::Reopen => {
                store.close(world.document());
                world.generation += 1;
                world.revision = 0;
            }
            Step::Supersede => {
                store.abandon();
            }
        }
        for producer in [
            Producer::Outline,
            Producer::Server(ServerId::new(world.server)),
        ] {
            let served = store.lookup(
                world.document(),
                producer,
                BufferRevision::new(world.revision),
            );
            let expected = list(world.document(), world.revision, producer);
            if served.is_some_and(|served| *served != expected) {
                return Err(format!(
                    "after step {at} of {sequence:?}: {producer:?} served a list of another state"
                ));
            }
        }
    }
    Ok(())
}

/// Every sequence of `depth` steps, in order; the first failure, if any.
fn explore(depth: usize, accept: Accept) -> Result<usize, String> {
    let mut sequence = vec![Step::Edit; depth];
    let mut indexes = vec![0usize; depth];
    let mut explored = 0;
    loop {
        for (step, index) in sequence.iter_mut().zip(&indexes) {
            *step = STEPS[*index];
        }
        run(&sequence, accept)?;
        explored += 1;
        let mut place = 0;
        loop {
            if place == depth {
                return Ok(explored);
            }
            indexes[place] += 1;
            if indexes[place] < STEPS.len() {
                break;
            }
            indexes[place] = 0;
            place += 1;
        }
    }
}

const DEPTH: usize = 6;

#[test]
fn no_order_of_edits_builds_and_deliveries_serves_another_states_list() {
    let explored = explore(DEPTH, honest).unwrap();
    assert_eq!(explored, STEPS.len().pow(DEPTH as u32));
}

#[test]
fn the_explorer_catches_a_result_filed_under_the_revision_at_delivery() {
    let found = explore(DEPTH, stamped_at_delivery).unwrap_err();
    assert!(found.contains("served a list of another state"), "{found}");
}

#[test]
fn the_explorer_catches_a_result_filed_under_the_wrong_producer() {
    let found = explore(DEPTH, filed_as_the_server).unwrap_err();
    assert!(found.contains("served a list of another state"), "{found}");
}

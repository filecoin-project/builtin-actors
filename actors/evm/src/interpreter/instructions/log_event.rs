use crate::interpreter::instructions::memory::get_memory_region;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::ActorError;
use fvm_ipld_encoding::{to_vec, BytesSer, RawBytes};
use fvm_shared::event::{Entry, Flags};
use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

/// The event key for the Ethereum log data.
const EVENT_DATA_KEY: &str = "d";

/// The event keys for the Ethereum log topics.
const EVENT_TOPIC_KEYS: &[&str] = &["t1", "t2", "t3", "t4"];

#[inline]
pub fn log(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
    num_topics: usize,
    mem_index: U256,
    size: U256,
    topics: &[U256],
) -> Result<(), ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("log called while read-only".into()));
    }

    // Handle the data.
    // Passing in a zero-sized memory region omits the data key entirely.
    // LOG0 + a zero-sized memory region emits an event with no entries whatsoever. In this case,
    // the FVM will record a hollow event carrying only the emitter actor ID.
    let region = get_memory_region(&mut state.memory, mem_index, size)?;

    // Extract the topics. Prefer to allocate an extra item than to incur in the cost of a
    // decision based on the size of the data.
    let mut entries: Vec<Entry> = Vec::with_capacity(num_topics + 1);
    for i in 0..num_topics {
        let key = EVENT_TOPIC_KEYS[i];
        let topic = topics[i];
        let entry = Entry {
            flags: Flags::FLAG_INDEXED_ALL,
            key: (*key).to_owned(),
            value: to_vec(&topic)?.into(), // U256 serializes as a byte string.
        };
        entries.push(entry);
    }

    // Skip adding the data if it's zero-sized.
    if let Some(r) = region {
        let data = state.memory[r.offset..r.offset + r.size.get()].to_vec();
        let entry = Entry {
            flags: Flags::FLAG_INDEXED_ALL,
            key: EVENT_DATA_KEY.to_owned(),
            value: RawBytes::serialize(BytesSer(&data))?,
        };
        entries.push(entry);
    }

    system.rt.emit_event(&entries.into())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use fil_actors_evm_shared::uints::U256;
    use fvm_ipld_encoding::{to_vec, BytesSer, RawBytes};
    use fvm_shared::event::{ActorEvent, Entry, Flags};

    use super::{EVENT_DATA_KEY, EVENT_TOPIC_KEYS};
    use crate::evm_unit_test;

    #[test]
    fn test_log0() {
        evm_unit_test! {
            (rt) {
                let mut data = [0u8; 32];
                data[28] = 0xCA;
                data[29] = 0xFE;
                data[30] = 0xBA;
                data[31] = 0xBE;
                rt.expect_emitted_event(
                    ActorEvent::from(vec![Entry{
                        flags: Flags::FLAG_INDEXED_ALL,
                        key: EVENT_DATA_KEY.to_owned(),
                        value: RawBytes::serialize(BytesSer(&data)).unwrap(),
                    }])
                );
            }
            (m) {
                PUSH4; 0xCA; 0xFE; 0xBA; 0xBE;
                PUSH0;
                MSTORE;
                PUSH1; 0x20;
                PUSH0;
                LOG0;
            }

            let result = m.execute();
            assert!(result.is_ok(), "execution step failed");
        };
    }

    #[test]
    fn test_log1() {
        evm_unit_test! {
            (rt) {
                let t1 = U256::from(0x01);
                let mut data = [0u8; 32];
                data[28] = 0xCA;
                data[29] = 0xFE;
                data[30] = 0xBA;
                data[31] = 0xBE;
                rt.expect_emitted_event(
                    ActorEvent::from(vec![
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[0].to_owned(),
                            value: to_vec(&t1).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key: EVENT_DATA_KEY.to_owned(),
                            value: RawBytes::serialize(BytesSer(&data)).unwrap(),
                        }
                    ])
                );
            }
            (m) {
                PUSH4; 0xCA; 0xFE; 0xBA; 0xBE;
                PUSH0;
                MSTORE;
                PUSH1; 0x01;
                PUSH1; 0x20;
                PUSH0;
                LOG1;
            }

            let result = m.execute();
            assert!(result.is_ok(), "execution step failed");
        };
    }

    #[test]
    fn test_log2() {
        evm_unit_test! {
            (rt) {
                let t1 = U256::from(0x01);
                let t2 = U256::from(0x02);
                let mut data = [0u8; 32];
                data[28] = 0xCA;
                data[29] = 0xFE;
                data[30] = 0xBA;
                data[31] = 0xBE;
                rt.expect_emitted_event(
                    ActorEvent::from(vec![
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[0].to_owned(),
                            value: to_vec(&t1).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[1].to_owned(),
                            value: to_vec(&t2).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key: EVENT_DATA_KEY.to_owned(),
                            value: RawBytes::serialize(BytesSer(&data)).unwrap(),
                        }
                    ])
                );
            }
            (m) {
                PUSH4; 0xCA; 0xFE; 0xBA; 0xBE;
                PUSH0;
                MSTORE;
                PUSH1; 0x02;
                PUSH1; 0x01;
                PUSH1; 0x20;
                PUSH0;
                LOG2;
            }

            let result = m.execute();
            assert!(result.is_ok(), "execution step failed");
        };
    }

    #[test]
    fn test_log3() {
        evm_unit_test! {
            (rt) {
                let t1 = U256::from(0x01);
                let t2 = U256::from(0x02);
                let t3 = U256::from(0x03);
                let mut data = [0u8; 32];
                data[28] = 0xCA;
                data[29] = 0xFE;
                data[30] = 0xBA;
                data[31] = 0xBE;
                rt.expect_emitted_event(
                    ActorEvent::from(vec![
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[0].to_owned(),
                            value: to_vec(&t1).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[1].to_owned(),
                            value: to_vec(&t2).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[2].to_owned(),
                            value: to_vec(&t3).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key: EVENT_DATA_KEY.to_owned(),
                            value: RawBytes::serialize(BytesSer(&data)).unwrap(),
                        }
                    ])
                );
            }
            (m) {
                PUSH4; 0xCA; 0xFE; 0xBA; 0xBE;
                PUSH0;
                MSTORE;
                PUSH1; 0x03;
                PUSH1; 0x02;
                PUSH1; 0x01;
                PUSH1; 0x20;
                PUSH0;
                LOG3;
            }

            let result = m.execute();
            assert!(result.is_ok(), "execution step failed");
        };
    }

    #[test]
    fn test_log4() {
        evm_unit_test! {
            (rt) {
                let t1 = U256::from(0x01);
                let t2 = U256::from(0x02);
                let t3 = U256::from(0x03);
                let t4 = U256::from(0x04);
                let mut data = [0u8; 32];
                data[28] = 0xCA;
                data[29] = 0xFE;
                data[30] = 0xBA;
                data[31] = 0xBE;
                rt.expect_emitted_event(
                    ActorEvent::from(vec![
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[0].to_owned(),
                            value: to_vec(&t1).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[1].to_owned(),
                            value: to_vec(&t2).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[2].to_owned(),
                            value: to_vec(&t3).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key:  EVENT_TOPIC_KEYS[3].to_owned(),
                            value: to_vec(&t4).unwrap().into(),
                        },
                        Entry{
                            flags: Flags::FLAG_INDEXED_ALL,
                            key: EVENT_DATA_KEY.to_owned(),
                            value: RawBytes::serialize(BytesSer(&data)).unwrap(),
                        }
                    ])
                );
            }

            (m) {
                PUSH4; 0xCA; 0xFE; 0xBA; 0xBE;
                PUSH0;
                MSTORE;
                PUSH1; 0x04;
                PUSH1; 0x03;
                PUSH1; 0x02;
                PUSH1; 0x01;
                PUSH1; 0x20;
                PUSH0;
                LOG4;
            }

            let result = m.execute();
            assert!(result.is_ok(), "execution step failed");
        };
    }
}

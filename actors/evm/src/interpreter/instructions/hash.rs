use fil_actors_runtime::ActorError;

use {
    super::memory::get_memory_region,
    crate::interpreter::{ExecutionState, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_shared::crypto::hash::SupportedHashes,
};

pub fn keccak256(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
    index: U256,
    size: U256,
) -> Result<U256, ActorError> {
    let region = get_memory_region(&mut state.memory, index, size)?;

    let (buf, size) = system.rt.hash_64(
        SupportedHashes::Keccak256,
        if let Some(region) = region {
            &state.memory[region.offset..region.offset + region.size.get()]
        } else {
            &[]
        },
    );

    Ok(U256::from_big_endian(&buf[..size]))
}

#[cfg(test)]
mod test {
    use fil_actors_runtime::runtime::Primitives;
    use rand::{distributions::Standard, Rng};

    use crate::{evm_unit_test, interpreter::U256, BytecodeHash};

    #[test]
    fn keccak256_rand() {
        let r = &mut rand::thread_rng();

        for _ in 0..32 {
            let data_max = 0x1000; // 4Kib
            let len = r.gen_range(32..data_max);
            let v: Vec<u8> = r.sample_iter(Standard).take(len).collect();
            let [a, b] = u16::try_from(len).unwrap().to_be_bytes();
            evm_unit_test! {
                (rt, m) {
                    PUSH2;
                    {a};
                    {b};
                    PUSH0;
                    KECCAK256;
                }

                let expect = &m.system.rt.hash_64(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &v).0[..32];

                m.state.memory.grow(len);
                m.state.memory[..len].copy_from_slice(&v);
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");

                assert_eq!(m.state.stack.pop().unwrap(), U256::from(expect));
            };
        }
    }

    #[test]
    fn keccak256_ext() {
        for (input, expect) in
            [([0xfe].as_slice(), BytecodeHash::NATIVE_ACTOR), (&[], BytecodeHash::EMPTY)]
        {
            evm_unit_test! {
                (rt, m) {
                    PUSH1;
                    {input.len() as u8};
                    PUSH0;
                    KECCAK256;
                }
                m.state.memory.grow(input.len());
                m.state.memory[..input.len()].copy_from_slice(input);
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");

                assert_eq!(m.state.stack.pop().unwrap(), U256::from(expect));
            };
        }
    }
}

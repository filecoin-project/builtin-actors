use fil_actors_integration_tests::tests::{nested_multisig_test, nested_multisig_direct_proposal_test};
use vm_api::new_with_singletons;

fn main() {
    println!("Testing nested multisig functionality...");
    
    // Create a VM instance
    let vm = new_with_singletons(vm_api::DefaultFilecoinKernel);
    
    // Run the first test
    println!("Running nested_multisig_test...");
    nested_multisig_test(&vm);
    println!("✓ nested_multisig_test passed!");
    
    // Run the second test  
    println!("Running nested_multisig_direct_proposal_test...");
    nested_multisig_direct_proposal_test(&vm);
    println!("✓ nested_multisig_direct_proposal_test passed!");
    
    println!("All tests passed! Nested multisig works correctly.");
}
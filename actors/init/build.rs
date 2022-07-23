static NETWORKS: &[(&str, &[&str])] = &[
    ("mainnet", &[]),
    ("caterpillarnet", &[]),
    ("butterflynet", &[]),
    ("calibrationnet", &[]),
    ("devnet", &[]),
    ("devnet-m2-native", &["m2-native"]),
    ("testing", &[]),
    ("testing-fake-proofs", &[]),
];
const NETWORK_ENV: &str = "BUILD_FIL_NETWORK";

fn main() {
    let network = std::env::var(NETWORK_ENV).ok();
    println!("cargo:rerun-if-env-changed={}", NETWORK_ENV);

    let network = network.as_deref().unwrap_or("mainnet");
    let features = NETWORKS.iter().find(|(k, _)| k == &network).expect("unknown network").1;
    for feature in features {
        println!("cargo:rustc-cfg=feature=\"{}\"", feature);
    }
}

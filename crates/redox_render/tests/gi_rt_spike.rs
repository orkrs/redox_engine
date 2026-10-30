//! GI-05 spike: are hardware ray queries available through wgpu on the target adapter?
//! Prints the answer; the test only fails when the adapter query itself breaks.

mod common;

#[test]
fn report_ray_tracing_support() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let features = adapter.features();
    let ray_query = features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
    eprintln!(
        "GI-05 spike: adapter {:?}, EXPERIMENTAL_RAY_QUERY = {ray_query}",
        adapter.get_info().name
    );
}

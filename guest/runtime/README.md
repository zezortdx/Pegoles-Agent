# guest/runtime — Pegoles Guest Runtime (future)

Agent I/O agent inside the VM: screenshots, input injection, guest file/shell execution, and the Core↔Guest protocol endpoint.

Phase 1: no code. The host side speaks only `pegoles-protocol::ComputerAction` to the `MockComputerBackend`.

// The app's whole IPC surface, in one list. `build.rs` turns it into the
// ACL manifest (one `allow-<command>` permission per command), `lib.rs`
// into the invoke handler, and the tests hold the capabilities to it.
//
// `release`: exactly the commands the product webview calls
// (`api` in src/lib/tauri.ts), granted by capabilities/default.json.
// `debug`: the Design Lab's raw action and script commands, base-image
// preparation and diagnostics. They are compiled, registered and granted
// (debug-capabilities/design-lab.json, added at startup) only with debug
// assertions, never in a release build.
ipc_commands! {
    release: [
        get_status,
        create_computer,
        start_computer,
        pause_computer,
        resume_computer,
        stop_computer,
        reset_computer,
        destroy_computer,
        list_events,
        read_boot_log,
        get_host_capabilities,
        accessibility_display,
        suggested_effects,
        display_set_geometry,
        display_detach,
        take_control,
        return_control,
        create_task,
        list_tasks,
        run_task,
        cancel_task,
        get_model_settings,
        enter_api_key,
        clear_api_key,
        set_model_settings,
        get_intelligence,
        set_provider,
        install_local_model,
        cancel_local_model_install,
        remove_local_model,
        cancel_agent_input,
        capture_screen,
        install_computer_image,
        cancel_computer_image_install,
        get_onboarding,
        set_onboarding_step,
        finish_onboarding,
        system_check,
        fix_virtualization,
        restart_to_finish_setup,
    ],
    debug: [
        prepare_image,
        execute_action,
        run_input_script,
        demo_script_steps,
        input_status,
        pump,
        get_image_status,
        guest_info,
        guest_ping,
        suggested_config,
        input_audit,
    ],
}

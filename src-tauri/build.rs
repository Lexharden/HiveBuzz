fn main() {
    // `option_env!` se evalúa al compilar: si cambian los Client ID integrados hay que recompilar.
    println!("cargo:rerun-if-env-changed=HIVEBUZZ_SPOTIFY_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=HIVEBUZZ_TWITCH_CLIENT_ID");
    // El icono de la app (ventana, bandeja, .exe) se incrusta al compilar: si cambia, hay que recompilar.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}

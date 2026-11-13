# Pi aislado con Bubblewrap

Launcher **escrito en Rust** para ejecutar la distribución autónoma de Pi en un
filesystem mínimo en Linux. Un único binario, sin Python ni runtime de Rust en el
equipo receptor. El launcher no instala dependencias ni modifica tu configuración
de escritorio. El aislamiento lo aplica Bubblewrap al proceso de Pi y a todos sus
descendientes, no un prompt o una extensión. Docker queda para otra versión.

## Instalación

Instala la última release para **Linux x86_64** con:

```sh
curl -fsSL https://raw.githubusercontent.com/ewiggin/pi-sandbox/main/install.sh | sh
```

El instalador descarga el binario, verifica su SHA-256 y lo instala en
`~/.local/bin/pi-sandbox`, sin `sudo`, Rust ni Python. Después comprueba Pi y
Bubblewrap; si faltan, muestra cómo instalarlos, pero no los instala por ti.
Necesita `curl`, `sha256sum` y las herramientas básicas del sistema.

Si `~/.local/bin` no está en tu `PATH`, añade esto a tu sesión (y al archivo de
inicio de tu shell si quieres conservarlo):

```sh
export PATH="$HOME/.local/bin:$PATH"
pi-sandbox --check
```

Para actualizar, repite el comando de instalación. Se sustituye el ejecutable
atómicamente y se conservan los logins y sesiones. Para una versión concreta o
un directorio distinto, puedes pasar variables al instalador:

```sh
curl -fsSL https://raw.githubusercontent.com/ewiggin/pi-sandbox/main/install.sh |
  PI_SANDBOX_VERSION=v0.1.0 PI_SANDBOX_INSTALL_DIR="$HOME/.local/bin" sh
```

El comando ejecuta código del repositorio: úsalo solo si confías en él. Puedes
[revisar install.sh](https://github.com/ewiggin/pi-sandbox/blob/main/install.sh)
antes de ejecutarlo. El checksum detecta corrupción, pero no sustituye una firma.
La instalación requiere una release publicada con el binario y su checksum.

## Requisitos de ejecución

- Linux con namespaces de usuario habilitados.
- `bubblewrap` con soporte para namespaces de usuario y `--disable-userns`.
- Pi autónomo instalado y disponible en `PATH` (la instalación actual con mise
  funciona). El wrapper npm todavía no está soportado.
- Herramientas del sistema en `/usr`; no se montan tus runtimes privados de mise,
  pyenv, npm, etc. Los proyectos que los necesiten requerirán añadirlos de forma
  deliberada o usar posteriormente una imagen Docker.

En Arch, si falta Bubblewrap: `sudo pacman -S bubblewrap`.

El launcher comprueba **Pi y Bubblewrap automáticamente antes de abrir el proyecto
o crear estado**. Si faltan ambos, muestra los dos errores y cómo instalarlos;
no instala nada automáticamente ni ejecuta Pi fuera del sandbox.

También puedes comprobarlos sin iniciar el sandbox:

```bash
pi-sandbox --check
pi-sandbox --check --pi-dir /ruta/a/distribucion/pi
```

Devuelve código `0` si encuentra los dos ejecutables y Pi tiene formato ELF
(distribución autónoma), o `1` si falta alguno/no es válido. Muestra sus rutas.
La comprobación no ejecuta Pi ni Bubblewrap: no certifica sus versiones, integridad
o funcionamiento; los permisos de namespaces y la compatibilidad de Bubblewrap
se comprueban al iniciar el aislamiento. `--help` y `--version` del launcher no
necesitan estas dependencias.

## Compilar e instalar

Solo el equipo que compila necesita Rust/Cargo 1.85+ y un linker C. Las dependencias
Rust quedan fijadas en `Cargo.lock` y se incorporan al binario; no se descargan al
ejecutarlo.

```bash
cargo build --release --locked
./target/release/pi-sandbox --help

# Opcional: instalar fuera de los proyectos accesibles al agente
install -Dm755 target/release/pi-sandbox "$HOME/.local/bin/pi-sandbox"
```

El build normal usa la libc del equipo compilador; para distribuir entre distros
con distintas versiones de glibc, usa el build estático de abajo. No hace falta
instalar Rust/Cargo ni Python en el equipo receptor.

### Binario estático distribuible (musl)

Con una toolchain gestionada por rustup, prepara el target de tu arquitectura:

```bash
rustup target add x86_64-unknown-linux-musl
./scripts/build-release.sh
```

El target también puede venir con la toolchain de tu distribución. Se ha probado
el build musl x86_64 en esta máquina. El script crea:

```text
dist/pi-sandbox-x86_64-unknown-linux-musl/pi-sandbox
dist/pi-sandbox-x86_64-unknown-linux-musl.tar.gz
dist/pi-sandbox-x86_64-unknown-linux-musl.tar.gz.sha256
```

Para ARM64 se selecciona `aarch64-unknown-linux-musl` en un host ARM64. Puedes
establecer `TARGET` explícitamente; la compilación cruzada requiere además el
linker/target apropiado. El binario es específico de Linux y de su arquitectura,
no universal. Aunque el launcher sea estático, **Bubblewrap, Pi y las herramientas
del sistema siguen siendo dependencias externas**.

Para verificar y extraer el paquete recibido:

```bash
sha256sum -c pi-sandbox-x86_64-unknown-linux-musl.tar.gz.sha256
tar -xzf pi-sandbox-x86_64-unknown-linux-musl.tar.gz
./pi-sandbox-x86_64-unknown-linux-musl/pi-sandbox --help
```

El checksum detecta alteraciones accidentales; no sustituye una firma ni una
fuente de distribución de confianza. No extraigas un paquete sobre un binario
que esté ejecutándose; instala actualizaciones con las sesiones detenidas.

### Publicación automática en GitHub

El workflow `.github/workflows/release.yml` se ejecuta al subir **cualquier tag**.
Compila en Ubuntu el binario estático Linux x86_64 y crea una GitHub Release con:

- `pi-sandbox-linux-x86_64`: ejecutable suelto.
- `pi-sandbox-linux-x86_64.sha256`: checksum del ejecutable.
- `pi-sandbox-x86_64-unknown-linux-musl.tar.gz`: binario y README.
- `pi-sandbox-x86_64-unknown-linux-musl.tar.gz.sha256`: checksum del paquete.

Antes de publicar comprueba formato, Clippy, pruebas unitarias, pruebas offline
del instalador, enlace estático, arranque básico, diagnóstico de dependencias
ausentes y checksums. Las pruebas de
integración completas necesitan Pi y namespaces de usuario y se ejecutan localmente
como se indica más abajo; el workflow no instala ni ejecuta Pi fuera del sandbox.

Cuando el repositorio esté en GitHub, con Actions habilitado:

```bash
# Actualiza antes Cargo.toml/Cargo.lock si cambia la versión del launcher.
git tag v0.1.0
git push origin v0.1.0
```

No necesita un PAT ni secretos adicionales: utiliza `GITHUB_TOKEN`, con permiso
`contents: write` solo en el job de publicación. Las políticas de la organización
pueden restringirlo. Reejecutar el workflow actualiza los assets de la misma release.
Solo publica x86_64 por ahora; no se generan binarios ARM64 en este workflow.

Para usar el ejecutable suelto descargado, verifica su checksum y dale permisos:

```bash
sha256sum -c pi-sandbox-linux-x86_64.sha256
chmod +x pi-sandbox-linux-x86_64
./pi-sandbox-linux-x86_64 --check
```

## Uso

Después de instalar el binario en `PATH` (o usando su ruta completa):

```bash
# Prueba sin red ni credenciales
pi-sandbox --project /ruta/al/proyecto -- --version

# Pi interactivo con acceso a Internet y autenticación propia
pi-sandbox --project /ruta/al/proyecto --network

# Autenticación independiente, sin abrir un navegador desde el sandbox
pi-sandbox --project /ruta/al/proyecto --network -- auth login openai-codex --no-browser

# Selección de modelo u otros argumentos de Pi después de --
pi-sandbox --project /ruta/al/proyecto --network -- --model openai-codex/gpt-5.4

# Inspección manual del mismo entorno, sin arrancar Pi
pi-sandbox --project /ruta/al/proyecto --exec /bin/bash --noprofile --norc
```

Sin `--project` se usa el directorio actual. Dentro siempre se llama `/workspace`.
El modelo del ejemplo debe estar disponible en tu cuenta; puedes elegirlo con
`/model`. También puedes iniciar Pi y usar `/login` dentro. Si un OAuth necesita
callback, pega manualmente el código/URL cuando Pi lo pida.

### Compartir el login entre proyectos

Usa **el mismo `--auth-profile NOMBRE`** en distintos proyectos para iniciar sesión
una sola vez y reutilizar las credenciales:

```bash
# Primer uso: inicia sesión dentro del sandbox
pi-sandbox --project ~/c/proyecto-a --auth-profile personal --network
# Dentro de Pi: /login

# Otro proyecto, mismo login
pi-sandbox --project ~/c/proyecto-b --auth-profile personal --network

# Perfil independiente para otra cuenta
pi-sandbox --project ~/c/trabajo --auth-profile trabajo --network
```

También puedes autenticarte sin abrir una sesión de agente:

```bash
pi-sandbox --auth-profile personal --network -- auth login openai-codex --no-browser
```

Se comparte un directorio dedicado a credenciales (`auth.json` y sus bloqueos).
Los agentes también pueden añadir archivos a ese directorio compartido. El HOME,
los historiales, la configuración, las extensiones y las skills continúan separados
por proyecto, reglas de exclusión y perfil. Cambiar las exclusiones mantiene el
login del perfil, pero utiliza un HOME/historial diferente.

Los perfiles viven en:

```text
~/.local/state/pi-sandbox/auth-profiles/<nombre>/auth.json
```

`--state-root` cambia también la ubicación de estos perfiles. Los nombres admiten
1–64 caracteres ASCII: letras, números, puntos, guiones y guiones bajos, empezando
por letra o número. Se crea un perfil vacío si no existe; no se importan claves del
HOME real ni de los entornos privados anteriores. El primer login del perfil es
independiente de los logins que ya tuvieras en cada proyecto.

**Seguridad:** todos los agentes que usen el mismo perfil pueden leer, cambiar o
borrar sus credenciales. El perfil es un dominio de confianza: no lo compartas con
repositorios no confiables. Cerrar sesión o revocar credenciales afecta a todos los
proyectos del perfil. Mantén cuentas/claves dedicadas y con límites.

Sin `--auth-profile`, el comportamiento anterior no cambia: credenciales privadas
por proyecto y exclusiones, sin acceso a los perfiles compartidos. Tampoco se
sobrescriben sus logins o historiales al probar un perfil compartido. La lectura,
escritura y el bloqueo compartido de credenciales se han probado con Pi 1.5.2.

### Excluir rutas

Por defecto se incluye toda la carpeta actual. `--project` cambia esa carpeta
base; **no acumula proyectos**. Dentro de ella se suman las reglas de
`.pi-sandboxignore` y todos los `--ignore`:

```bash
# Carpeta actual, salvo las exclusiones del archivo y estas dos adicionales
pi-sandbox --ignore .env --ignore secrets/ --network

# Otra carpeta base: se lee /ruta/al/proyecto/.pi-sandboxignore
pi-sandbox --project /ruta/al/proyecto --ignore '*.pem' --network
```

Ejemplo de `.pi-sandboxignore` (una regla por línea):

```text
# Nombres excluidos a cualquier profundidad
.env
secrets/
*.pem

# Rutas relativas a la raíz del proyecto
config/private.json
src/**/*.key
```

- Se ignoran líneas vacías y comentarios que empiezan por `#`. Se recortan
  espacios de principio y final. No se interpreta `.gitignore`.
- Un patrón sin `/` se aplica al nombre en cualquier profundidad. `secrets/`
  excluye directorios llamados `secrets`, pero no archivos con ese nombre.
- Con `/`, la ruta parte de la raíz del proyecto. `*`, `?` y `[abc]` afectan
  a un componente; `**` como componente permite cero o más niveles.
  Por ejemplo, `src/*.pem` no incluye `src/deep/key.pem`; `src/**/*.pem`, sí.
- Pon los globs entre comillas al usar `--ignore` para que no los expanda tu shell.
- No hay negaciones `!`, rutas absolutas, componentes `..`, ni sintaxis completa
  de gitignore. Una regla inválida impide arrancar, en lugar de ignorarse.

**Las exclusiones son montajes de protección**, no una recomendación a Pi.
Se conserva el bind RW del proyecto para que las ediciones, archivos nuevos y
renombrados de las rutas permitidas sigan escribiéndose directamente en el host.
Sobre las coincidencias se montan sustitutos: archivos sin contenido accesible y
árboles privados inaccesibles y de solo lectura. **Los nombres pueden seguir
apareciendo** en listados, pero su contenido original no puede leerse ni escribirse.
Los hardlinks dentro del proyecto que apuntan a un archivo excluido se protegen
igualmente, incluso si ese archivo está dentro de un directorio excluido.

Limitaciones de esta primera implementación:

- Las coincidencias se calculan **al arrancar**, no continuamente. Los patrones
  sin coincidencias no reservan rutas futuras. No añadas/muevas secretos ni
  cambies el árbol desde el host mientras Pi esté ejecutándose: reinicia el
  sandbox para recalcular la protección. Un patrón no protege copias con otros
  nombres, backups, contenido previamente guardado en otros archivos o en Git.
- No se siguen symlinks al buscar coincidencias. Una regla que excluye un symlink
  o puede atravesarlo hace fallar el arranque: excluye su destino real o el
  directorio padre. Los patrones recursivos pueden rechazar symlinks de forma
  conservadora. No se monta a ciegas sobre el destino de un enlace.
- Las rutas protegidas son puntos de montaje: no podrás borrarlas/reemplazarlas
  ni eliminar un directorio que las contenga desde el sandbox. Los archivos
  permitidos no están montados individualmente y mantienen sus guardados normales.
- Se recorre el árbol, también los directorios excluidos, para detectar aliases
  por hardlink. Un árbol sin permisos de lectura falla de forma cerrada; los
  repositorios grandes pueden tardar más en arrancar. Con exclusiones activas,
  los nombres de entradas deben ser UTF-8; si no lo son, el arranque falla en vez
  de dejar un posible secreto sin proteger. La ruta base sí admite bytes no UTF-8.
- Cambiar las reglas crea un HOME/estado independiente para no revelar sesiones
  previas que pudieron leer datos ahora excluidos. Sin `--auth-profile`, tendrás
  que iniciar sesión de nuevo con esa combinación de reglas. Si eliges un perfil
  compartido, conserva su login explícitamente, pero no el historial anterior. Quitar una regla vuelve al estado anterior
  correspondiente; las exclusiones no borran datos del host ni revocan secretos.

No se copian `~/.pi`, sesiones, extensiones, skills, tokens ni claves del host.
Sin un perfil compartido, el estado es independiente **por ruta absoluta de proyecto
y reglas de exclusión** y persiste en:

```text
~/.local/state/pi-sandbox/<sha256-de-la-identidad>/home/
```

Dentro, ese directorio es `/home/sandbox`; Pi usa `~/.pi/agent`. Dos ejecuciones
sobre el mismo proyecto y con las mismas reglas comparten estado; proyectos distintos
no. Mover un proyecto cambia su identidad. La versión Rust conserva el formato
de identidad de la versión Python anterior: los logins y sesiones existentes se
reutilizan sin copiarlos ni migrarlos. Para reiniciarlo, elimina su directorio de estado **desde el
host y con las sesiones detenidas**. `--state-root` permite cambiar la ubicación,
pero no puede estar dentro del proyecto ni contenerlo.

Para pasar una API key explícitamente, sin ponerla en los argumentos:

```bash
read -rs -p 'API key: ' OPENAI_API_KEY; echo
export OPENAI_API_KEY
pi-sandbox --project /ruta/al/proyecto --network --env OPENAI_API_KEY
unset OPENAI_API_KEY
```

**La variable y cualquier credencial guardada dentro del sandbox son accesibles
para los agentes.** Este launcher protege el host, no oculta a Pi sus propias
credenciales. Usa credenciales dedicadas, con límites, y revócalas si es necesario.
`--env NAME` es una concesión explícita; no pases agentes SSH ni configuración
sensible indiscriminadamente.

Para otra distribución autónoma: `--pi-dir /ruta/al/directorio/que/contiene/pi`.
Actualiza Pi desde el host: su instalación está montada en solo lectura.

## Qué se permite

| Recurso | Acceso |
| --- | --- |
| Proyecto elegido | Lectura y escritura, salvo rutas protegidas por exclusiones |
| HOME privado de este proyecto y perfil | Lectura y escritura persistente |
| Credenciales compartidas | Solo si eliges explícitamente `--auth-profile NOMBRE` |
| `/tmp`, `/run` privados | Efímeros |
| `/usr`, compatibilidad `/bin`/`lib`, instalación de Pi | Solo lectura |
| Certificados del sistema y configuración mínima | Solo lectura |
| HOME real, otros proyectos, `/etc` completo, `/sys` | No montados |
| Agente SSH, D-Bus, Wayland/X11, socket Docker del host | No montados por el launcher |
| Entorno del host y descriptores adicionales | No heredados, salvo variables explícitas |
| Procesos del host | Ocultos mediante namespace PID y `/proc` propio |
| Red | Bloqueada por defecto; `--network` comparte la del host |

Se eliminan capabilities, se bloquean nuevos user namespaces, se crea una sesión
separada de terminal y se termina el sandbox si desaparece su proceso padre.
Si falla Bubblewrap, **no hay fallback a ejecutar Pi fuera del sandbox**.
`PI_OFFLINE` solo reduce actividad automática de Pi: el bloqueo real de red viene
del namespace de red.

## Límites importantes

- **`--network` NO es una allowlist del proveedor.** Permite Internet, LAN,
  localhost y sockets Unix abstractos del namespace de red del host. Un agente
  puede exfiltrar cualquier dato visible y acceder a servicios locales sin
  autenticación. Una siguiente fase debería usar proxy/egress controlado desde
  fuera del sandbox; configurar `HTTP_PROXY` por sí solo no obliga a usarlo.
- Sin red, los proveedores remotos no funcionan. Es el modo conservador para
  inspecciones y pruebas; para una sesión normal debes optar por `--network`.
- El agente puede borrar o modificar **las rutas no excluidas del proyecto**,
  `.git` incluido si no se excluye. Usa copias, commits o worktrees desechables.
  Es preferible no poner secretos en el proyecto; si existen, exclúyelos antes de
  arrancar. `.env`, claves, backups y archivos ocultos son visibles por defecto.
- No selecciones una carpeta amplia como proyecto. Evita sockets del host,
  hardlinks a archivos externos y submontajes sensibles dentro del proyecto:
  un bind mount no sanea su contenido. Los symlinks a rutas externas no montadas
  no permiten escapar, pero los hardlinks ya son el mismo archivo.
- El kernel y los binarios del sistema siguen siendo compartidos. No es una VM,
  no hay perfil seccomp adicional ni cuotas de CPU/RAM/disco; un proceso hostil
  todavía puede agotar recursos o explotar vulnerabilidades del kernel.
- La terminal y stdin/stdout siguen conectados al host. La sesión separada reduce
  riesgos clásicos de inyección de terminal, pero no filtra secuencias de escape,
  portapapeles OSC ni contenido malicioso mostrado por la aplicación.
- La configuración/extensiones del proyecto y el estado privado pueden ejecutar
  código **dentro** del sandbox y persistir entre sesiones. No ejecutes ese estado
  ni los scripts producidos por el agente fuera del sandbox sin revisarlos.

### Mantener el launcher fuera del alcance del agente

Si lanzas Pi sobre **este repositorio**, podrá editar su código, scripts, pruebas
y binarios generados. Compilar o ejecutar después una copia modificada desde el
host sería una vía de compromiso.
Para uso habitual, copia una versión revisada a una ruta de confianza fuera de los
proyectos expuestos, por ejemplo:

```bash
install -Dm755 target/release/pi-sandbox "$HOME/.local/bin/pi-sandbox"
cd /ruta/al/proyecto
pi-sandbox --network
```

No se realiza esta instalación automáticamente. No reutilices un launcher que un
agente haya podido modificar sin revisarlo.

## Pruebas reales

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked

# Ejecutar la misma suite de integración contra el binario estático distribuible
PI_SANDBOX_TEST_BIN="$PWD/dist/pi-sandbox-x86_64-unknown-linux-musl/pi-sandbox" \
  cargo test --locked --tests
```

Todas las pruebas están escritas en Rust; usan shells del sistema para las sondas
internas. No requieren Python, credenciales ni llamadas al proveedor. Comprueban escritura
en el proyecto, persistencia y separación del HOME, enlaces externos inaccesibles,
entorno y FDs limpios, procesos ocultos, montajes de solo lectura, red bloqueada
frente a un servidor local, red explícita, bloqueo de namespaces anidados, códigos
de salida y arranque del binario real de Pi. También verifican la suma de
exclusiones CLI/archivo, los globs, la escritura persistente en rutas permitidas,
los aliases por symlink/hardlink, los renombrados de padres, las reglas inválidas
y la separación de estado por exclusiones. Se comprueban los perfiles compartidos,
la separación del resto del HOME y los bloqueos del archivo de credenciales usando
Pi real, sin llamar a ningún proveedor. Se comprueba también la compatibilidad
del estado con el launcher anterior y la ausencia de fallback si Bubblewrap falla.
Necesitan un host Linux donde Bubblewrap funcione y Pi esté instalado; no se omiten
silenciosamente si el aislamiento falla.

## Próximas fases

1. Red de salida restringida desde fuera del sandbox y credenciales en un gateway.
2. Límites de recursos mediante cgroups/systemd y/o un perfil seccomp probado.
3. Backend Docker con imagen de herramientas reproducible, usuario no root,
   sin socket Docker montado y las mismas pruebas de aislamiento.

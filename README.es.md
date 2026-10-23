# RCode

Un agente de programación local: el agente se ejecuta dentro de este proceso y habla con el proveedor de modelos que configures. Las sesiones se guardan en este dispositivo. Las solicitudes al modelo se envían al proveedor configurado; no necesitas una cuenta de RCode.

*[English](README.md) | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | Español*

![App de escritorio de RCode](docs/media/readme/app-screenshot.jpg)

## App de escritorio

Descarga la última versión para tu plataforma desde [GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest):

- **macOS** — `rcode-<version>-macos-arm64.dmg`
- **Windows** — `rcode-<version>-windows-x86_64-setup.exe`
- **Linux** — `rcode-<version>-linux-<arch>.tar.gz` y luego ejecuta el `install.sh` que incluye

No necesitas una cuenta de RCode. El acceso al modelo y las actualizaciones usan la red; las sesiones se guardan localmente. La app se actualiza sola.

## Servicio sin interfaz y conexiones remotas

Un servidor Linux puede ejecutar el motor sin pantalla. Los clientes de escritorio se conectan con su dirección y una clave API. Configura la clave antes de la primera instalación:

```bash
read -r -s -p 'API Key: ' RCODE_API_KEY
printf '\n'
export RCODE_API_KEY
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

La clave debe tener al menos 8 caracteres ASCII, solo letras y números, e incluir ambos. El puerto externo predeterminado es `27655`; configura `RCODE_REMOTE_PORT` antes de instalar para cambiarlo. Si hay un servicio de usuario de systemd disponible, el instalador configura el servicio en segundo plano e intenta mantenerlo activo al cerrar sesión. Sigue sus indicaciones si el servicio no arranca.

Si ya tienes el binario, puedes ejecutar el motor directamente. Es obligatorio indicar `--api-key` o `RCODE_API_KEY`; `--port` permite cambiar el puerto:

```bash
rcode headless --api-key '<tu-clave-api>'
rcode headless --api-key '<tu-clave-api>' --port 27656
```

Con esas variables de entorno configuradas, instala y administra un servicio permanente:

```bash
rcode daemon install
rcode status      # estado del motor
rcode update      # actualizar a la última versión
rcode daemon start|stop|restart|status
```

En la app de escritorio, abre **Ajustes → Dispositivos**, añade un host e introduce `ws://dirección-del-servidor:27655` y la misma clave API. Un escritorio también puede activar **Acceso externo** allí: al activarlo por primera vez se genera y habilita una clave aleatoria de 12 caracteres alfanuméricos. Puedes sustituirla por una clave propia que cumpla las reglas anteriores y guardarla.

Una desconexión de red o el cierre del cliente remoto no detiene las ejecuciones que el motor remoto ya haya aceptado. Cerrar la app de escritorio que aloja el motor detiene el motor de esa máquina; usa el servicio sin interfaz para un servidor permanente. La [app móvil Flutter](apps/mobile/README.md) ofrece Android e iOS con el mismo protocolo WebSocket directo y sigue las pantallas de la app iOS anterior. Han pasado las pruebas de widgets y de integración con el motor real. La [validación de desarrollo](docs/development-validation.md) registra los paquetes Android arm64/x86_64, las pruebas en simulador y dispositivo Android arm64, y la compilación Xcode sin firma con pruebas en simulador iOS. Siguen pendientes iPhone/iPad físicos, firma, TestFlight y proveedores de modelos reales.

---

¿Quieres contribuir o te interesa saber cómo funciona? [Pregúntale a DeepWiki](https://deepwiki.com/chengliang4810/RCode) o consulta [ARCHITECTURE.md](ARCHITECTURE.md).

Publicado bajo la [licencia MIT](LICENSE).

# RCode

Un agente de programación local: el agente se ejecuta dentro de este proceso y habla con el proveedor de modelos que configures. Las sesiones se guardan en este dispositivo. Las solicitudes al modelo se envían al proveedor configurado; no necesitas una cuenta de RCode.

*[English](README.md) | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | Español*

![App de escritorio de RCode](docs/media/readme/app-screenshot.jpg)

## App de escritorio

Descarga la última versión para tu plataforma desde [GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest):

- **macOS** — `rcode-macos-arm64.dmg`
- **Windows** — `rcode-windows-x86_64-setup.exe`
- **Linux** — `rcode-linux-<arch>.tar.gz` y luego ejecuta el `install.sh` que incluye

No necesitas una cuenta de RCode. El acceso al modelo y las actualizaciones usan la red; las sesiones se guardan localmente. La app se actualiza sola.

## Modo headless (CLI)

Para servidores y otras máquinas sin pantalla, como un VPS que mantiene los agentes en ejecución después de cerrar tu portátil. Solo Linux:

```bash
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

El instalador arranca el motor como un servicio en segundo plano que sobrevive a los reinicios.

```bash
rcode status      # estado del motor
rcode update      # actualizar a la última versión
rcode daemon start|stop|restart|status
```

---

¿Quieres contribuir o te interesa saber cómo funciona? [Pregúntale a DeepWiki](https://deepwiki.com/chengliang4810/RCode) o consulta [ARCHITECTURE.md](ARCHITECTURE.md).

Publicado bajo la [licencia MIT](LICENSE).

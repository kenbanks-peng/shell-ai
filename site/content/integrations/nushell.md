---
title: Nushell integration
shell: nushell
weight: 4
configFile: config.nu
initCommand: $env.config.keybindings ++= (shell-ai init nu | from json)
exampleCommand: echo $numbers.0
description: Use Nushell's structured-data syntax and add the generated keybindings to your configuration.
---

Enable shell-ai in Nushell.

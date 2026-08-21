export interface ShellGuide {
  id: "bash" | "zsh" | "fish" | "nushell";
  name: string;
  configFile: string;
  description: string;
  initCommand: string;
  example: string;
}

export const shellGuides: ShellGuide[] = [
  {
    id: "bash",
    name: "Bash",
    configFile: ".bashrc",
    description: "Use Bash array syntax without translating your request into shell syntax first.",
    initCommand: 'eval "$(shell-ai init bash)"',
    example: 'echo "${numbers[0]}"',
  },
  {
    id: "zsh",
    name: "Zsh",
    configFile: ".zshrc",
    description: "Use Zsh-aware suggestions, including its one-based array indexing.",
    initCommand: 'eval "$(shell-ai init zsh)"',
    example: 'echo "${numbers[1]}"',
  },
  {
    id: "fish",
    name: "Fish",
    configFile: "config.fish",
    description: "Use Fish-native commands and source the generated integration directly.",
    initCommand: "shell-ai init fish | source",
    example: "echo $numbers[1]",
  },
  {
    id: "nushell",
    name: "Nushell",
    configFile: "config.nu",
    description: "Use Nushell's structured-data syntax and add the generated keybindings to your configuration.",
    initCommand: "$env.config.keybindings ++= (shell-ai init nu | from json)",
    example: "echo $numbers.0",
  },
];

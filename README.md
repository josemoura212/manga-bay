# Manga Bay - Rede P2P de Mangás

**Manga Bay** é uma rede peer-to-peer (P2P) distribuída projetada para a hospedagem e distribuição eficiente de mangás, manhuas e manhwas. O sistema opera sem servidores centrais, utilizando a contribuição dos usuários para manter a rede ativa e saudável.

## 🚀 Funcionalidades

*   **Rede Totalmente Descentralizada**: Baseada em `libp2p`, sem ponto único de falha.
*   **Sistema de Ratio**: Incentiva o compartilhamento. Usuários devem contribuir (upload) para consumir (download).
*   **Gerenciamento de Recursos**: O nó monitora e limita o uso de CPU, Memória e Disco a 10% do sistema, garantindo que não afete o desempenho do seu computador.
*   **Descoberta Automática**: Utiliza DHT (Kademlia) e mDNS para encontrar outros pares na rede.
*   **Persistência Inteligente**: Salva pares conhecidos para facilitar a reconexão (Bootstrap/DNS Nodes).

## 🛠️ Pré-requisitos

*   **Rust**: Você precisa ter o Rust e o Cargo instalados.
    *   Instalação: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` (ou visite [rustup.rs](https://rustup.rs))

## 📦 Instalação e Execução

1.  Clone o repositório:
    ```bash
    git clone https://github.com/seu-usuario/manga-bay.git
    cd manga-bay
    ```

2.  Compile e execute o projeto:
    ```bash
    cargo run --release
    ```

### Argumentos da Linha de Comando

*   `--port <PORTA>`: Porta para a API HTTP (padrão: 3000). A porta P2P será `PORTA + 1`.
*   `--data-dir <CAMINHO>`: Diretório onde os dados (banco de dados e chunks) serão salvos (padrão: `./data`).
*   `--bootstrap-peers <MULTIADDR>`: Endereço de um nó existente para entrar na rede.

## 📖 Como Usar

### 1. Iniciar o Primeiro Nó (Bootstrap)

Este nó servirá como ponto de entrada para outros.

```bash
cargo run -- --port 3000 --data-dir ./data/node1
```

*Observe o log para ver o `PeerId` e o endereço de escuta (ex: `/ip4/127.0.0.1/tcp/3001/p2p/12D3...`).*

### 2. Iniciar um Segundo Nó

Em outro terminal, inicie um segundo nó conectando-se ao primeiro.

```bash
cargo run -- --port 3002 --data-dir ./data/node2 --bootstrap-peers /ip4/127.0.0.1/tcp/3001/p2p/<PEER_ID_DO_NODE1>
```

### 3. API HTTP

O nó expõe uma API REST para interagir com a biblioteca local.

#### Adicionar um Mangá (Ingestão)

Agora suporta envio de múltiplas páginas (imagens em Base64) e metadados detalhados.

```bash
curl -X POST http://localhost:3000/mangas \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Capítulo 1",
    "author": "Eiichiro Oda",
    "series_code": "OP-001",
    "series_title": "One Piece",
    "alternative_titles": ["One Piece (PT-BR)", "ワンピース"],
    "language": "pt-br",
    "pages": [
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
    ]
  }'
```
*(Nota: `pages` é uma lista de strings Base64. O servidor irá criar um arquivo ZIP/CBZ automaticamente.)*

#### Listar Mangás

```bash
curl http://localhost:3000/mangas
```

## 🏗️ Arquitetura

*   **Storage**: SQLite para metadados, Sistema de Arquivos para blocos (chunks).
*   **P2P**: Gossipsub para anúncios, Request-Response para transferência de dados.
*   **Segurança**: Comunicação criptografada com Noise Protocol.

## 🤝 Contribuição

Contribuições são bem-vindas! Sinta-se à vontade para abrir Issues ou Pull Requests.

## 📄 Licença

Este projeto é open-source e distribuído sob a licença MIT.

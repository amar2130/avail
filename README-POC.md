# Avail Cosmos POC

For the purpose of the POC, `README-POC.md` file and `contract` and `web` folders are added.

## Wasmd node installation

> git clone https://github.com/CosmWasm/wasmd.git  
> cd wasmd  
> git checkout v0.30.0  
> make install

### Environment

> export CHAIN_ID="avail-poc"

### Init the node

> wasmd init avail-poc-node --chain-id $CHAIN_ID  
> wasmd keys add main  
> wasmd add-genesis-account $(wasmd keys show main -a) 10000000000stake  
> wasmd gentx main 1000000000stake --chain-id $CHAIN_ID  
> wasmd collect-gentxs  
> wasmd validate-genesis

### Compiling smart contract (from the avail-light repository)

> cd contract  
> RUSTFLAGS='-C link-arg=-s' cargo wasm

For some reason, rust version 1.7 doesn't work, so if contract fails to deploy, run:
> RUSTFLAGS='-C link-arg=-s' cargo +1.65.0-x86_64-unknown-linux-gnu wasm

### Running the node and creating the contract

> wasmd start  
> wasmd tx wasm store target/wasm32-unknown-unknown/release/contract.wasm --from main --chain-id $CHAIN_ID --gas-prices 0.025stake --gas auto --gas-adjustment 1.3 -y -b block

Check if contract is deployed:

> wasmd query wasm list-code

Instantiate contract:

> export INIT='{"balances":[["sonic","100"],["tails","100"]]}'  
> wasmd tx wasm instantiate 1 $INIT --from main --chain-id $CHAIN_ID --label "avail-poc" --no-admin

Get code and contract IDs:

> export CODE_ID=$(wasmd query wasm list-code --output json | jq -r '.code_infos[0].code_id')  
> export CONTRACT=$(wasmd query wasm list-contract-by-code $CODE_ID --output json | jq -r '.contracts[-1]')  
> echo $CONTRACT

## Avail node 

> git clone https://github.com/availproject/avail.git  
> git checkout v1.4.0-rc4  
> cargo run --release -p data-avail -- --dev --tmp

## Avail light bootstrap node

> git clone https://github.com/availproject/avail-light-bootstrap.git  
> cargo run --release

## Avail light node

> git clone https://github.com/availproject/avail-light.git  
> git checkout aterentic/cosmos-poc  
> cargo run --release -- -c config-poc.yaml

Application will crash but it will create configuration file with defaults. Replace following keys:

> libp2p_port = '37001'  
> bootstraps = [["{bootstrap-local-peer-id}","/ip4/127.0.0.1/udp/37000"]]  
> contract = '{contract-address}'  
> sender_mnemonic = '{sender-mnemonic}'  
> app_id = 1

- `bootstrap-local-peer-id` can be found in bootstrap node logs in form like `12D3KooWCF23YWVMsX6vQa1m6tyU66L3fJJeRcQqgnHgqgzxLAPE`
- `contract-addres` is result of former `echo $CONTRACT` command
- `sender-mnemonic` is result of `wasmd keys add main` command

Run application again:

> cargo run --release -- -c config-poc.yaml

## Web application

Source is in `/web` folder, there is no need to compile anything.  
To run, just start web server in `/web` folder with `python3 -m http.server 8000`

At this point, everything should be up and running, access application at http:\\localhnost:8000

## Flow

![avail-poc](https://user-images.githubusercontent.com/97872690/234561034-08250766-78e2-46b3-9dae-898a9a04181d.png)

- Web application queries current state (and "block height", to track progress):


![Image](https://user-images.githubusercontent.com/97872690/234544221-0db92bcc-6066-4a08-962f-52cb0a3471ff.png)


- Web application posts transaction to transaction queue and receives response with simulated state after transaction execution:



![Image](https://user-images.githubusercontent.com/97872690/234544267-cf20ec73-c8f8-4c62-b620-58c367ee3a14.png)


- Response contains current "block height" used to track progess (button outline is orange if transactions are not finalized)


![Image](https://user-images.githubusercontent.com/97872690/234544313-de9cfa28-665b-4453-8312-095398e09b7e.png)


- Transaction queue batches transactions and posts batch to avail (and commits them to node) in regular intervals.
- When block is final in avail, and batch is verified by application client, custom application is notified.
- Custom application broadcast batch to full nodes (not implemented for the POC).
- Web application polls custom application for "block height" and updates UI (button outline becomes blue again).


![Image](https://user-images.githubusercontent.com/97872690/234544372-8b7f78dc-1ac3-468c-a5ab-f017d4998616.png)

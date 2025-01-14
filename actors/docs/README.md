## APIs between user programmed actors and built-in actors

[FIP-0050](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0050.md) exports a stable subset of the built-in actor methods, along with some new methods, for invocation around storage, market, miner and etc by user actors. 

The method number of the exported methods follows [FRC-0042](https://github.com/filecoin-project/FIPs/blob/master/FRCs/frc-0042.md) and can be called by any type of caller. Read-only method will not modify the state, it is similar as view functions in solidity.

This API docs intend to provide information about how to use those exported APIs in user-defined actors. You can find the docs of the exported  APIs and their usage in this folder.


## Exported API list

| actor      | method                                                       | method number | ReadOnly           |
| ---------- | ------------------------------------------------------------ | ------------- | ------------------ |
| account    | [AuthenticateMessage](account.md#AuthenticateMessage)        | 2643134072    | :heavy_check_mark: |
| datacap    | [Name](datacap.md#Name)                                      | 48890204      | :heavy_check_mark: |
| datacap    | [Symbol](datacap.md#Symbol)                                  | 2061153854    | :heavy_check_mark: |
| datacap    | [Granularity](datacap.md#Granularity)                        | 3936767397    | :heavy_check_mark: |
| datacap    | [TotalSupply](datacap.md#TotalSupply)                        | 114981429     | :heavy_check_mark: |
| datacap    | [Balance](datacap.md#Balance)                                | 3261979605    | :heavy_check_mark: |
| datacap    | [Transfer](datacap.md#Transfer)                              | 80475954      |                    |
| datacap    | [TransferFrom](datacap.md#TransferFrom)                      | 3621052141    |                    |
| datacap    | [IncreaseAllowance](datacap.md#IncreaseAllowance)            | 1777121560    |                    |
| datacap    | [DecreaseAllowance](datacap.md#DecreaseAllowance)            | 1529376545    |                    |
| datacap    | [RevokeAllowance](datacap.md#RevokeAllowance)                | 2765635761    |                    |
| datacap    | [Burn](datacap.md#Burn)                                      | 1434719642    |                    |
| datacap    | [BurnFrom](datacap.md#BurnFrom)                              | 2979674018    |                    |
| datacap    | [Allowance](datacap.md#Allowance)                            | 4205072950    | :heavy_check_mark: |
| datacap    | [Mint](datacap.md#mint)                                      | 116935346     |                    |
| datacap    | [Destroy](datacap.md#Destroy)                                | 2624896501    |                    |
| ethaccount | [AuthenticateMessage](ethaccount.md#AuthenticateMessage)     | 2643134072    | :heavy_check_mark: |
| evm        | InvokeEVM                                                    | 3844450837    |                    |
| market     | [AddBalance](market.md#AddBalance)                           | 822473126     |                    |
| market     | [WithdrawBalance](market.md#WithdrawBalance)                 | 2280458852    |                    |
| market     | [PublishStorageDeals](market.md#PublishStorageDeals)         | 2236929350    |                    |
| market     | [GetBalance](market.md#GetBalance)                           | 726108461     | :heavy_check_mark: |
| market     | [GetDealDataCommitment](market.md#GetDealDataCommitment)     | 1157985802    | :heavy_check_mark: |
| market     | [GetDealClient](market.md#GetDealClient)                     | 128053329     | :heavy_check_mark: |
| market     | [GetDealProvider](market.md#GetDealProvider)                 | 935081690     | :heavy_check_mark: |
| market     | [GetDealLabel](market.md#GetDealLabel)                       | 46363526      | :heavy_check_mark: |
| market     | [GetDealTerm](market.md#GetDealTerm)                         | 163777312     | :heavy_check_mark: |
| market     | [GetDealTotalPrice](market.md#GetDealTotalPrice)             | 4287162428    | :heavy_check_mark: |
| market     | [GetDealClientCollateral](market.md#GetDealClientCollateral) | 200567895     | :heavy_check_mark: |
| market     | [GetDealProviderCollateral](market.md#GetDealProviderCollateral) | 2986712137    | :heavy_check_mark: |
| market     | [GetDealVerified](market.md#GetDealVerified)                 | 2627389465    | :heavy_check_mark: |
| market     | [GetDealActivation](market.md#GetDealActivation)             | 2567238399    | :heavy_check_mark: |
| miner      | [ChangeWorkerAddress](miner.md#ChangeWorkerAddress)          | 1010589339    |                    |
| miner      | [ChangePeerID](miner.md#ChangePeerID)                        | 1236548004    |                    |
| miner      | [WithdrawBalance](miner.md#WithdrawBalance)                  | 2280458852    |                    |
| miner      | [ChangeMultiaddrs](miner.md#ChangeMultiaddrs)                | 1063480576    |                    |
| miner      | [ConfirmChangeWorkerAddress](miner.md#ConfirmChangeWorkerAddress) | 2354970453    |                    |
| miner      | [RepayDebt](miner.md#RepayDebt)                              | 3665352697    |                    |
| miner      | [ChangeOwnerAddress](miner.md#ChangeOwnerAddress)            | 1010589339    |                    |
| miner      | [ChangeBeneficiary](miner.md#ChangeBeneficiary)              | 1570634796    |                    |
| miner      | [GetBeneficiary](miner.md#GetBeneficiary)                    | 4158972569    | :heavy_check_mark: |
| miner      | [GetOwner](miner.md#GetOwner)                                | 3275365574    | :heavy_check_mark: |
| miner      | [IsControllingAddress](miner.md#IsControllingAddress)        | 348244887     | :heavy_check_mark: |
| miner      | [GetSectorSize](miner.md#GetSectorSize)                      | 3858292296    | :heavy_check_mark: |
| miner      | [GetAvailableBalance](miner.md#GetAvailableBalance)          | 4026106874    | :heavy_check_mark: |
| miner      | [GetVestingFunds](miner.md#GetVestingFunds)                  | 1726876304    | :heavy_check_mark: |
| miner      | [GetPeerID](miner.md#GetPeerID)                              | 2812875329    | :heavy_check_mark: |
| miner      | [GetMultiaddrs](miner.md#GetMultiaddrs)                      | 1332909407    | :heavy_check_mark: |
| multisig   | [Receive](multisig.md#Receive)                               | 3726118371    |                    |
| power      | [CreateMiner](power.md#CreateMiner)                          | 1173380165    |                    |
| power      | [NetworkRawPower](power.md#NetworkRawPower)                  | 931722534     | :heavy_check_mark: |
| power      | [MinerRawPower](power.md#MinerRawPower)                      | 3753401894    | :heavy_check_mark: |
| power      | [MinerCount](power.md#MinerCount)                            | 3753401894    | :heavy_check_mark: |
| power      | [MinerConsensusCount](power.md#MinerConsensusCount)          | 196739875     | :heavy_check_mark: |
| verifreg   | [AddVerifiedClient](verifreg.md#AddVerifiedClient)           | 3916220144    |                    |
| verifreg   | [RemoveExpiredAllocations](verifreg.md#RemoveExpiredAllocations) | 2873373899    |                    |
| verifreg   | [GetClaims](verifreg.md#GetClaims)                           | 2199871187    | :heavy_check_mark: |
| verifreg   | [ExtendClaimTerms](verifreg.md#ExtendClaimTerms)             | 1752273514    |                    |
| verifreg   | [RemoveExpiredClaims](verifreg.md#RemoveExpiredClaims)       | 2873373899    |                    |
| verifreg   | [Receive](verifreg.md#Receive)                               | 3726118371    |                    |
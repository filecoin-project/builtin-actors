## APIs between user programmed actors and built-in actors

[FIP-0050](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0050.md) exports a stable subset of the built-in actor methods, along with some new methods, for invocation around storage, market, miner and etc by user actors. 

The method number of the exported methods follows [FRC-0042](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0050.md) and can be called by any type of caller.

This API docs intend to provide information about how to use those exported APIs in user-defined actors. You can find the docs of the exported  APIs and their usage in this folder.


## Exported API list

|actor   	|method   	|method number   	|
|---	|---	|---	|
|account   	|AuthenticateMessage   	|2643134072   	|
|datacap  	|Mint   	|116935346   	|
|datacap   	|Destroy   	|2624896501   	|
|datacap   	|Name   	|48890204   	|
|datacap   	|Symbol   	|2061153854   	|
|datacap   	|Granularity   	|3936767397   	|
|datacap   	|TotalSupply   	|114981429   	|
|datacap   	|Balance   	|3261979605   	|
|datacap   	|Transfer   	|80475954   	|
|datacap   	|TransferFrom   	|3621052141   	|
|datacap   	|IncreaseAllowance   	|1777121560   	|
|datacap   	|DecreaseAllowance   	|1529376545   	|
|datacap   	|RevokeAllowance   	|2765635761   	|
|datacap   	|Burn   	|1434719642   	|
|datacap   	|BurnFrom   	|2979674018   	|
|datacap   	|Allowance   	|4205072950   	|
|ethaccount   	|AuthenticateMessage   	|2643134072   	|
|evm   	|InvokeEVM   	|3844450837   	|
|market   	|AddBalance   	|822473126   	|
|market   	|WithdrawBalance   	|2280458852   	|
|market   	|PublishStorageDeals   	|2236929350   	|
|market   	|GetBalance   	|726108461   	|
|market   	|GetDealDataCommitment   	|1157985802   	|
|market   	|GetDealClient   	|128053329   	|
|market   	|GetDealProvider   	|935081690   	|
|market   	|GetDealLabel   	|46363526   	|
|market   	|GetDealTerm   	|163777312   	|
|market   	|GetDealTotalPrice   	|4287162428   	|
|market   	|GetDealClientCollateral   	|200567895   	|
|market   	|GetDealProviderCollateral   	|2986712137   	|
|market   	|GetDealVerified   	|2627389465   	|
|market   	|GetDealActivation   	|2567238399   	|
|miner   	|ChangeWorkerAddress   	|1010589339   	|
|miner   	|ChangePeerID   	|1236548004   	|
|miner   	|WithdrawBalance   	|2280458852   	|
|miner   	|ChangeMultiaddrs   	|1063480576   	|
|miner   	|ConfirmChangeWorkerAddress   	|2354970453   	|
|miner   	|RepayDebt   	|3665352697   	|
|miner   	|ChangeOwnerAddress   	|1010589339   	|
|miner   	|ChangeBeneficiary   	|1570634796   	|
|miner   	|GetBeneficiary   	|4158972569   	|
|miner   	|GetOwner   	|3275365574   	|
|miner   	|IsControllingAddress   	|348244887   	|
|miner   	|GetSectorSize   	|3858292296   	|
|miner   	|GetAvailableBalance   	|4026106874   	|
|miner   	|GetVestingFunds   	|1726876304   	|
|miner   	|GetPeerID   	|2812875329   	|
|miner   	|GetMultiaddrs   	|1332909407   	|
|multisig   	|Receive   	|3726118371   	|
|power   	|CreateMiner   	|1173380165   	|
|power   	|NetworkRawPower   	|931722534   	|
|power   	|MinerRawPower   	|3753401894   	|
|power   	|MinerCount   	|3753401894   	|
|power   	|MinerConsensusCount   	|196739875   	|
|verifreg   	|AddVerifiedClient   	|3916220144   	|
|verifreg   	|RemoveExpiredAllocations   	|2873373899   	|
|verifreg   	|GetClaims   	|2199871187   	|
|verifreg   	|ExtendClaimTerms   	|1752273514   	|
|verifreg   	|RemoveExpiredClaims   	|2873373899   	|
|verifreg   	|Receive   	|3726118371   	|
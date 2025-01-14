# ethAccount Actor

The account actor is responsible for Ethereum comparable account. If you want to call these methods in your smart  contracts, you need to specify method number of that method you want to invoke. Please refer the each method for its method number.

#### **AuthenticateMessage**

```go
func AuthenticateMessage(params AuthenticateMessageParams) EmptyValue ()
```

Authenticates whether the provided signature is valid for the provided message. 

`uint` AuthenticateMessageMethodNum = 2643134072.

**Params**:

+  `struct` AuthenticateMessageParams
+ `bytes` Signature - it should be a raw byte of signature, NOT a serialized signature object with a signatureType.
  
+ ` bytes` Message -  The message which is signed by the corresponding account address.

**Results**:

+  `struct` EmptyValue.

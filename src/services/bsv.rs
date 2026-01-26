use bs58;
use rand::rngs::OsRng;
use ripemd::Ripemd160;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

pub struct BsvService {
    _private_key: Option<String>,
    pub fee_rate: f64,
}

impl BsvService {
    pub fn new(private_key: Option<String>, fee_rate: f64) -> Self {
        BsvService {
            _private_key: private_key,
            fee_rate,
        }
    }

    /// Generate a new keypair and return (WIF private key, address)
    pub fn generate_keypair() -> (String, String) {
        let secp = Secp256k1::new();
        let (secret_key, public_key) = secp.generate_keypair(&mut OsRng);

        let wif = Self::secret_key_to_wif(&secret_key);
        let address = Self::public_key_to_address(&public_key);

        (wif, address)
    }

    /// Convert WIF to SecretKey
    pub fn wif_to_secret_key(wif: &str) -> Result<SecretKey, String> {
        let decoded = bs58::decode(wif)
            .into_vec()
            .map_err(|e| format!("Invalid WIF: {}", e))?;

        if decoded.len() < 33 {
            return Err("WIF too short".to_string());
        }

        // Remove version byte (first) and checksum (last 4 bytes)
        // Also handle compressed key indicator (0x01 before checksum)
        let key_bytes = if decoded.len() == 38 {
            // Compressed: version(1) + key(32) + compressed(1) + checksum(4)
            &decoded[1..33]
        } else if decoded.len() == 37 {
            // Uncompressed: version(1) + key(32) + checksum(4)
            &decoded[1..33]
        } else {
            return Err(format!("Unexpected WIF length: {}", decoded.len()));
        };

        SecretKey::from_slice(key_bytes).map_err(|e| format!("Invalid key: {}", e))
    }

    /// Convert SecretKey to WIF (compressed)
    fn secret_key_to_wif(secret_key: &SecretKey) -> String {
        let mut data = vec![0x80]; // Mainnet version
        data.extend_from_slice(&secret_key[..]);
        data.push(0x01); // Compressed flag

        // Double SHA256 for checksum
        let hash1 = Sha256::digest(&data);
        let hash2 = Sha256::digest(&hash1);
        data.extend_from_slice(&hash2[..4]);

        bs58::encode(data).into_string()
    }

    /// Convert public key to BSV address
    fn public_key_to_address(public_key: &PublicKey) -> String {
        let serialized = public_key.serialize(); // Compressed

        // SHA256
        let sha256_hash = Sha256::digest(&serialized);

        // RIPEMD160
        let ripemd_hash = Ripemd160::digest(&sha256_hash);

        // Add version byte (0x00 for mainnet)
        let mut address_bytes = vec![0x00];
        address_bytes.extend_from_slice(&ripemd_hash);

        // Checksum
        let hash1 = Sha256::digest(&address_bytes);
        let hash2 = Sha256::digest(&hash1);
        address_bytes.extend_from_slice(&hash2[..4]);

        bs58::encode(address_bytes).into_string()
    }

    /// Get address from WIF
    pub fn wif_to_address(wif: &str) -> Result<String, String> {
        let secret_key = Self::wif_to_secret_key(wif)?;
        let secp = Secp256k1::new();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        Ok(Self::public_key_to_address(&public_key))
    }

    /// Calculate required satoshis for uploading data
    pub fn calculate_upload_cost(&self, data_size: usize) -> i64 {
        // Transaction overhead: ~150 bytes for inputs/outputs
        // Plus data size
        let tx_size = 150 + data_size;
        let fee = (tx_size as f64 * self.fee_rate).ceil() as i64;
        
        // Minimum 1 satoshi, plus some buffer
        std::cmp::max(fee + 1, 546) // 546 is dust limit
    }

    /// Create OP_RETURN script with data (legacy method)
    pub fn create_op_return_script(data_parts: &[&[u8]]) -> Vec<u8> {
        let mut script = Vec::new();

        // OP_FALSE OP_RETURN
        script.push(0x00); // OP_FALSE
        script.push(0x6a); // OP_RETURN

        for data in data_parts {
            Self::push_data(&mut script, data);
        }

        script
    }

    /// Create OP_FALSE OP_IF script for FLAC storage
    /// Format:
    ///   OP_FALSE (0x00)
    ///   OP_IF (0x63)
    ///     PUSHDATA <protocol identifier>  // "flacstore"
    ///     PUSHDATA <mime type>            // "audio/flac"
    ///     PUSHDATA <filename/metadata>    // JSON or string
    ///     PUSHDATA <data chunk 1>
    ///     PUSHDATA <data chunk 2>
    ///     ...
    ///   OP_ENDIF (0x68)
    pub fn create_flac_store_script(
        protocol: &[u8],
        mime_type: &[u8],
        metadata: &[u8],
        data_chunks: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut script = Vec::new();

        // OP_FALSE OP_IF
        script.push(0x00); // OP_FALSE
        script.push(0x63); // OP_IF

        // Protocol identifier
        Self::push_data(&mut script, protocol);

        // MIME type
        Self::push_data(&mut script, mime_type);

        // Metadata (filename, etc.)
        Self::push_data(&mut script, metadata);

        // Data chunks
        for chunk in data_chunks {
            Self::push_data(&mut script, chunk);
        }

        // OP_ENDIF
        script.push(0x68); // OP_ENDIF

        script
    }

    /// Split data into chunks suitable for PUSHDATA
    /// Maximum chunk size is 520 bytes for standard scripts,
    /// but BSV allows larger pushes. We'll use 100KB chunks for efficiency.
    pub fn split_into_chunks(data: &[u8], max_chunk_size: usize) -> Vec<Vec<u8>> {
        data.chunks(max_chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect()
    }

    /// Parse OP_FALSE OP_IF script and extract data
    /// Returns: (protocol, mime_type, metadata, data_chunks)
    pub fn parse_flac_store_script(script: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<Vec<u8>>), String> {
        if script.len() < 4 {
            return Err("Script too short".to_string());
        }

        // Check OP_FALSE OP_IF
        if script[0] != 0x00 || script[1] != 0x63 {
            return Err("Not a valid OP_FALSE OP_IF script".to_string());
        }

        let mut pos = 2;
        let mut data_parts: Vec<Vec<u8>> = Vec::new();

        while pos < script.len() {
            // Check for OP_ENDIF
            if script[pos] == 0x68 {
                break;
            }

            // Read PUSHDATA
            let (data, new_pos) = Self::read_push_data(script, pos)?;
            data_parts.push(data);
            pos = new_pos;
        }

        if data_parts.len() < 3 {
            return Err("Not enough data parts in script".to_string());
        }

        let protocol = data_parts.remove(0);
        let mime_type = data_parts.remove(0);
        let metadata = data_parts.remove(0);
        let data_chunks = data_parts;

        Ok((protocol, mime_type, metadata, data_chunks))
    }

    /// Read PUSHDATA from script at given position
    fn read_push_data(script: &[u8], pos: usize) -> Result<(Vec<u8>, usize), String> {
        if pos >= script.len() {
            return Err("Unexpected end of script".to_string());
        }

        let opcode = script[pos];
        let (data_len, data_start) = if opcode <= 75 {
            // Direct push
            (opcode as usize, pos + 1)
        } else if opcode == 0x4c {
            // OP_PUSHDATA1
            if pos + 1 >= script.len() {
                return Err("Missing length byte for OP_PUSHDATA1".to_string());
            }
            (script[pos + 1] as usize, pos + 2)
        } else if opcode == 0x4d {
            // OP_PUSHDATA2
            if pos + 2 >= script.len() {
                return Err("Missing length bytes for OP_PUSHDATA2".to_string());
            }
            let len = u16::from_le_bytes([script[pos + 1], script[pos + 2]]) as usize;
            (len, pos + 3)
        } else if opcode == 0x4e {
            // OP_PUSHDATA4
            if pos + 4 >= script.len() {
                return Err("Missing length bytes for OP_PUSHDATA4".to_string());
            }
            let len = u32::from_le_bytes([
                script[pos + 1],
                script[pos + 2],
                script[pos + 3],
                script[pos + 4],
            ]) as usize;
            (len, pos + 5)
        } else {
            return Err(format!("Unexpected opcode: 0x{:02x}", opcode));
        };

        if data_start + data_len > script.len() {
            return Err("Data extends beyond script".to_string());
        }

        let data = script[data_start..data_start + data_len].to_vec();
        Ok((data, data_start + data_len))
    }

    /// Push data with appropriate opcode
    pub fn push_data(script: &mut Vec<u8>, data: &[u8]) {
        let len = data.len();

        if len <= 75 {
            script.push(len as u8);
        } else if len <= 255 {
            script.push(0x4c); // OP_PUSHDATA1
            script.push(len as u8);
        } else if len <= 65535 {
            script.push(0x4d); // OP_PUSHDATA2
            script.extend_from_slice(&(len as u16).to_le_bytes());
        } else {
            script.push(0x4e); // OP_PUSHDATA4
            script.extend_from_slice(&(len as u32).to_le_bytes());
        }

        script.extend_from_slice(data);
    }

    /// Create P2PKH locking script
    pub fn create_p2pkh_script(address: &str) -> Result<Vec<u8>, String> {
        let decoded = bs58::decode(address)
            .into_vec()
            .map_err(|e| format!("Invalid address: {}", e))?;

        if decoded.len() != 25 {
            return Err("Invalid address length".to_string());
        }

        let pubkey_hash = &decoded[1..21];

        let mut script = Vec::new();
        script.push(0x76); // OP_DUP
        script.push(0xa9); // OP_HASH160
        script.push(0x14); // Push 20 bytes
        script.extend_from_slice(pubkey_hash);
        script.push(0x88); // OP_EQUALVERIFY
        script.push(0xac); // OP_CHECKSIG

        Ok(script)
    }

    /// Create a raw transaction
    pub fn create_transaction(
        &self,
        wif: &str,
        utxos: &[(String, u32, i64, Vec<u8>)], // (txid, vout, satoshis, scriptPubKey)
        outputs: &[(Vec<u8>, i64)],             // (scriptPubKey, satoshis)
    ) -> Result<String, String> {
        let secret_key = Self::wif_to_secret_key(wif)?;
        let secp = Secp256k1::new();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);

        let mut tx = Vec::new();

        // Version (4 bytes, little-endian)
        tx.extend_from_slice(&1u32.to_le_bytes());

        // Input count
        Self::write_varint(&mut tx, utxos.len() as u64);

        // Inputs (unsigned first)
        for (txid, vout, _, _) in utxos {
            // Previous txid (reversed)
            let txid_bytes = hex::decode(txid).map_err(|e| format!("Invalid txid: {}", e))?;
            let mut reversed = txid_bytes.clone();
            reversed.reverse();
            tx.extend_from_slice(&reversed);

            // Previous output index
            tx.extend_from_slice(&vout.to_le_bytes());

            // ScriptSig (empty for now)
            tx.push(0x00);

            // Sequence
            tx.extend_from_slice(&0xffffffffu32.to_le_bytes());
        }

        // Output count
        Self::write_varint(&mut tx, outputs.len() as u64);

        // Outputs
        for (script, satoshis) in outputs {
            tx.extend_from_slice(&satoshis.to_le_bytes());
            Self::write_varint(&mut tx, script.len() as u64);
            tx.extend_from_slice(script);
        }

        // Locktime
        tx.extend_from_slice(&0u32.to_le_bytes());

        // Now sign each input
        let mut signed_tx = Vec::new();
        signed_tx.extend_from_slice(&1u32.to_le_bytes()); // Version

        Self::write_varint(&mut signed_tx, utxos.len() as u64);

        for (i, (txid, vout, _, script_pubkey)) in utxos.iter().enumerate() {
            // Create sighash
            let sighash = self.create_sighash(&tx, i, script_pubkey, utxos, outputs)?;

            // Sign
            let message = Message::from_digest_slice(&sighash)
                .map_err(|e| format!("Invalid message: {}", e))?;
            let signature = secp.sign_ecdsa(&message, &secret_key);

            // Create scriptSig
            let mut sig_bytes = signature.serialize_der().to_vec();
            sig_bytes.push(0x41); // SIGHASH_ALL | SIGHASH_FORKID

            let pubkey_bytes = public_key.serialize();

            let mut script_sig = Vec::new();
            Self::push_data(&mut script_sig, &sig_bytes);
            Self::push_data(&mut script_sig, &pubkey_bytes);

            // Write input
            let txid_bytes = hex::decode(txid).map_err(|e| format!("Invalid txid: {}", e))?;
            let mut reversed = txid_bytes.clone();
            reversed.reverse();
            signed_tx.extend_from_slice(&reversed);
            signed_tx.extend_from_slice(&vout.to_le_bytes());
            Self::write_varint(&mut signed_tx, script_sig.len() as u64);
            signed_tx.extend_from_slice(&script_sig);
            signed_tx.extend_from_slice(&0xffffffffu32.to_le_bytes());
        }

        // Outputs
        Self::write_varint(&mut signed_tx, outputs.len() as u64);
        for (script, satoshis) in outputs {
            signed_tx.extend_from_slice(&satoshis.to_le_bytes());
            Self::write_varint(&mut signed_tx, script.len() as u64);
            signed_tx.extend_from_slice(script);
        }

        // Locktime
        signed_tx.extend_from_slice(&0u32.to_le_bytes());

        Ok(hex::encode(signed_tx))
    }

    fn create_sighash(
        &self,
        _tx: &[u8],
        input_index: usize,
        script_pubkey: &[u8],
        utxos: &[(String, u32, i64, Vec<u8>)],
        outputs: &[(Vec<u8>, i64)],
    ) -> Result<[u8; 32], String> {
        // BIP143 sighash for BSV (SIGHASH_ALL | SIGHASH_FORKID)
        let mut preimage = Vec::new();

        // 1. nVersion
        preimage.extend_from_slice(&1u32.to_le_bytes());

        // 2. hashPrevouts
        let mut prevouts = Vec::new();
        for (txid, vout, _, _) in utxos {
            let txid_bytes = hex::decode(txid).map_err(|e| format!("Invalid txid: {}", e))?;
            let mut reversed = txid_bytes.clone();
            reversed.reverse();
            prevouts.extend_from_slice(&reversed);
            prevouts.extend_from_slice(&vout.to_le_bytes());
        }
        let hash_prevouts = Self::double_sha256(&prevouts);
        preimage.extend_from_slice(&hash_prevouts);

        // 3. hashSequence
        let mut sequences = Vec::new();
        for _ in utxos {
            sequences.extend_from_slice(&0xffffffffu32.to_le_bytes());
        }
        let hash_sequence = Self::double_sha256(&sequences);
        preimage.extend_from_slice(&hash_sequence);

        // 4. outpoint
        let (txid, vout, _, _) = &utxos[input_index];
        let txid_bytes = hex::decode(txid).map_err(|e| format!("Invalid txid: {}", e))?;
        let mut reversed = txid_bytes.clone();
        reversed.reverse();
        preimage.extend_from_slice(&reversed);
        preimage.extend_from_slice(&vout.to_le_bytes());

        // 5. scriptCode
        Self::write_varint(&mut preimage, script_pubkey.len() as u64);
        preimage.extend_from_slice(script_pubkey);

        // 6. value
        let (_, _, satoshis, _) = &utxos[input_index];
        preimage.extend_from_slice(&satoshis.to_le_bytes());

        // 7. nSequence
        preimage.extend_from_slice(&0xffffffffu32.to_le_bytes());

        // 8. hashOutputs
        let mut outputs_data = Vec::new();
        for (script, sats) in outputs {
            outputs_data.extend_from_slice(&sats.to_le_bytes());
            Self::write_varint(&mut outputs_data, script.len() as u64);
            outputs_data.extend_from_slice(script);
        }
        let hash_outputs = Self::double_sha256(&outputs_data);
        preimage.extend_from_slice(&hash_outputs);

        // 9. nLocktime
        preimage.extend_from_slice(&0u32.to_le_bytes());

        // 10. sighash type (SIGHASH_ALL | SIGHASH_FORKID = 0x41)
        preimage.extend_from_slice(&0x41u32.to_le_bytes());

        Ok(Self::double_sha256(&preimage))
    }

    fn double_sha256(data: &[u8]) -> [u8; 32] {
        let hash1 = Sha256::digest(data);
        let hash2 = Sha256::digest(&hash1);
        let mut result = [0u8; 32];
        result.copy_from_slice(&hash2);
        result
    }

    fn write_varint(buf: &mut Vec<u8>, value: u64) {
        if value < 0xfd {
            buf.push(value as u8);
        } else if value <= 0xffff {
            buf.push(0xfd);
            buf.extend_from_slice(&(value as u16).to_le_bytes());
        } else if value <= 0xffffffff {
            buf.push(0xfe);
            buf.extend_from_slice(&(value as u32).to_le_bytes());
        } else {
            buf.push(0xff);
            buf.extend_from_slice(&value.to_le_bytes());
        }
    }
}

impl BsvService {
    /// Create FLAC chunk script for multi-transaction storage
    /// Format:
    ///   OP_FALSE (0x00)
    ///   OP_IF (0x63)
    ///     PUSHDATA "flacstore-chunk"
    ///     PUSHDATA <chunk_index as 4-byte LE>
    ///     PUSHDATA <data>
    ///   OP_ENDIF (0x68)
    pub fn create_flac_chunk_script(chunk_index: u32, _total_chunks: u32, data: &[u8]) -> Vec<u8> {
        let mut script = Vec::new();

        // OP_FALSE OP_IF
        script.push(0x00); // OP_FALSE
        script.push(0x63); // OP_IF

        // Protocol identifier
        Self::push_data(&mut script, b"flacstore-chunk");

        // Chunk index (as string for easier parsing)
        Self::push_data(&mut script, chunk_index.to_string().as_bytes());

        // Data
        Self::push_data(&mut script, data);

        // OP_ENDIF
        script.push(0x68);

        script
    }

    /// Create FLAC manifest script that references chunk transactions
    /// Format:
    ///   OP_FALSE (0x00)
    ///   OP_IF (0x63)
    ///     PUSHDATA "flacstore-manifest"
    ///     PUSHDATA <filename>
    ///     PUSHDATA <metadata JSON>
    ///     PUSHDATA <chunk_txid_1>
    ///     PUSHDATA <chunk_txid_2>
    ///     ...
    ///   OP_ENDIF (0x68)
    pub fn create_flac_manifest_script(
        filename: &str,
        file_size: usize,
        chunk_txids: &[String],
    ) -> Vec<u8> {
        let mut script = Vec::new();

        // OP_FALSE OP_IF
        script.push(0x00); // OP_FALSE
        script.push(0x63); // OP_IF

        // Protocol identifier
        Self::push_data(&mut script, b"flacstore-manifest");

        // Filename
        Self::push_data(&mut script, filename.as_bytes());

        // Metadata JSON
        let metadata = serde_json::json!({
            "size": file_size,
            "chunks": chunk_txids.len(),
            "version": "1.0",
            "mime": "audio/flac"
        }).to_string();
        Self::push_data(&mut script, metadata.as_bytes());

        // Chunk TXIDs
        for txid in chunk_txids {
            Self::push_data(&mut script, txid.as_bytes());
        }

        // OP_ENDIF
        script.push(0x68);

        script
    }
}


impl BsvService {
    /// Create a UTXO split transaction that divides one UTXO into multiple outputs
    /// This is used to prepare for multi-chunk uploads where each chunk needs its own UTXO
    /// 
    /// Returns: (raw_tx_hex, Vec<(output_index, satoshis)>)
    pub fn create_split_transaction(
        &self,
        wif: &str,
        input_txid: &str,
        input_vout: u32,
        input_satoshis: i64,
        script_pubkey: &[u8],
        num_outputs: usize,
        satoshis_per_output: i64,
    ) -> Result<String, String> {
        // Calculate total needed for outputs
        let total_output = satoshis_per_output * num_outputs as i64;
        
        // Estimate transaction size: ~10 bytes overhead + ~148 bytes per input + ~34 bytes per output
        let tx_size = 10 + 148 + (34 * num_outputs);
        let fee = (tx_size as f64 * self.fee_rate).ceil() as i64;
        
        if input_satoshis < total_output + fee {
            return Err(format!(
                "Insufficient funds for split: {} < {} + {}",
                input_satoshis, total_output, fee
            ));
        }
        
        // Create outputs
        let mut outputs: Vec<(Vec<u8>, i64)> = Vec::new();
        for _ in 0..num_outputs {
            outputs.push((script_pubkey.to_vec(), satoshis_per_output));
        }
        
        // Add change output if there's any remaining
        let change = input_satoshis - total_output - fee;
        if change > 546 {
            outputs.push((script_pubkey.to_vec(), change));
        }
        
        // Create the transaction
        let utxos = vec![(input_txid.to_string(), input_vout, input_satoshis, script_pubkey.to_vec())];
        self.create_transaction(wif, &utxos, &outputs)
    }
    
    /// Calculate the required satoshis per output for a split transaction
    /// Each output needs to cover the chunk transaction fee + 1 satoshi for data output
    pub fn calculate_chunk_output_satoshis(&self, chunk_size: usize) -> i64 {
        // Chunk transaction size: ~150 bytes overhead + chunk data size
        let chunk_tx_size = 200 + chunk_size;
        let chunk_fee = (chunk_tx_size as f64 * self.fee_rate).ceil() as i64;
        
        // Need fee + 1 satoshi for data output + small buffer
        chunk_fee + 10
    }
    
    /// Calculate total cost for multi-chunk upload
    /// Returns (total_satoshis, satoshis_per_chunk, num_chunks)
    pub fn calculate_multi_chunk_cost(&self, file_size: usize, chunk_size: usize) -> (i64, i64, usize) {
        let num_chunks = (file_size + chunk_size - 1) / chunk_size;
        let satoshis_per_chunk = self.calculate_chunk_output_satoshis(chunk_size);
        
        // Number of outputs in split transaction: num_chunks + 1 (for manifest)
        let num_outputs = num_chunks + 1;
        
        // Split transaction cost
        let split_tx_size = 10 + 148 + (34 * num_outputs);
        let split_fee = (split_tx_size as f64 * self.fee_rate).ceil() as i64;
        
        // Total output value needed for split transaction
        let split_output_total = satoshis_per_chunk * num_outputs as i64;
        
        // Total = split outputs + split fee
        let total = split_output_total + split_fee;
        
        (total, satoshis_per_chunk, num_chunks)
    }
}

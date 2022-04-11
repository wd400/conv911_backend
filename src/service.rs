use prost_types::Option;
use serde::{Serialize, Deserialize};
use jsonwebtoken::{encode, decode, Header, Algorithm, Validation, EncodingKey, DecodingKey, decode_header};
use tonic::{Request, Response, Status, codegen::http::request};
use crate::api::*;
use reqwest;
use std::{collections::HashMap, borrow::BorrowMut};
use sha2::{Sha256, Sha512, Digest};
//extern crate rusoto_core;
//extern crate rusoto_dynamodb;

use aws_sdk_dynamodb::{Client, Error, model::{AttributeValue, ReturnValue}, types::{SdkError, self}, error::{ConditionalCheckFailedException, PutItemError, conditional_check_failed_exception, PutItemErrorKind}};
 



#[derive(Debug, Serialize, Deserialize)]
struct JWTPayload {
    exp: i64,
    user_id: String,
    open_id: String
}

//#[derive(Default)]
pub struct  MyApi {
    pub jwt_key:EncodingKey,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub facebook_client_id: String,
    pub facebook_client_secret: String,
    pub dynamodb_client: Client,
    pub hash_salt:String,
}

const T :&str="lo";

const GRANT_TYPE :&str= "authorization_code";

fn test(){

}

#[derive(Deserialize)]
struct GoogleTokensJSON {
    id_token: String,
    expires_in: u32,
  //  id_token: String,
    scope: String,
    token_type: String,
    //this field is only present in this response if you set the access_type parameter to offline in the initial request to Google's authorization server. 
    //refresh_token: String

}


//google: only 'code' needed

#[tonic::async_trait]
impl v1::api_server::Api for MyApi {

    async fn login(&self, request: Request<login::LoginRequest>) -> Result<Response<login::LoginResponse>, Status> {
     //   println!("Got a request: {:#?}", &request);
        let request=request.get_ref();



        let third_party = login::LoginRequest::third_party(request);
        
        let open_id:String =
         if third_party==login::ThirdParty::Facebook {
        //    return Err(Status::new(tonic::Code::InvalidArgument, "not yet implemented"));

                    // https://developers.google.com/identity/protocols/oauth2/openid-connect#exchangecode
                    let client = reqwest::Client::new();

                    let facebook_request = client.post("https://graph.facebook.com/oauth/access_token")
                    .form(
                        &[
                            //safe? optimal?
                            ("code", request.code.clone()),
                            ("client_id",self.facebook_client_id.clone()),
                            ("client_secret",self.facebook_client_secret.clone()),
                            ("redirect_uri","https://example.com".into()),
                        ]).send().await;

                    print!("{:#?}",facebook_request);

                    let facebook_request = match facebook_request {
                        Ok(facebook_request) => facebook_request,
                        Err(_) => return Err(Status::new(tonic::Code::InvalidArgument, "oauth request error"))
                    };
        
                    let facebook_response = match facebook_request.json::<HashMap<String, String>>().await {
                        Ok(facebook_response) => facebook_response,
                        Err(_) => return Err(Status::new(tonic::Code::InvalidArgument, "oauth json error"))
                    };


                    match facebook_response.get("sub").cloned() {
                        Some(sub) => sub,
                        None => return Err(Status::new(tonic::Code::InvalidArgument, "oauth json error"))
                    }

            }
        else if third_party==login::ThirdParty::Google {

            // https://developers.google.com/identity/protocols/oauth2/openid-connect#exchangecode
            let client = reqwest::Client::new();
         
            
            let google_tokens = client.post("https://oauth2.googleapis.com/token")
            .form(
                &[
                    //safe? optimal?
                    ("code", request.code.clone()),
                    ("client_id",self.google_client_id.clone()),
                    ("client_secret",self.google_client_secret.clone()),
                    ("redirect_uri","https://example.com".into()),
                    ("grant_type",GRANT_TYPE.into())
                ]
            );
            // .mime_str("text/plain")?;
            
            let google_tokens = google_tokens.send().await;

            let google_tokens = match google_tokens {
                Ok(google_tokens) => google_tokens,
                Err(_) => return Err(Status::new(tonic::Code::InvalidArgument, "oauth request error"))
            };

            let google_tokens:GoogleTokensJSON = match google_tokens.json().await {
                Ok(google_tokens) => google_tokens,
                Err(_) => return Err(Status::new(tonic::Code::InvalidArgument, "oauth json error"))
            };

        let token_infos=client.get("https://oauth2.googleapis.com/tokeninfo?")
        .query(&[("id_token",google_tokens.id_token)])
        .send().await;
        
        let token_infos = match token_infos {
            Ok(token_infos) => token_infos,
            Err(_) => return Err(Status::new(tonic::Code::InvalidArgument, "tokeninfo request error"))
        };

        let token_infos = match token_infos.json::<HashMap<String, String>>().await {
            Ok(token_infos) => token_infos,
            Err(_) => return Err(Status::new(tonic::Code::InvalidArgument, "oauth json error"))
        };

        let sub = match token_infos.get("sub").cloned() {
            Some(sub) => sub,
            None => return Err(Status::new(tonic::Code::InvalidArgument, "oauth json error"))
        };
    
         sub
                
            }
        else {
                return Err(Status::new(tonic::Code::InvalidArgument, "third party is invalid"))
        };
        
        println!("openid: {}",open_id);
        

        //ConditionalCheckFailedException
        let res = self.dynamodb_client.put_item()
        .table_name("users")
        .item("openid",AttributeValue::S(String::from("0")))
        .item("amount",AttributeValue::N(String::from("0")))
        .condition_expression("attribute_not_exists(amount)")
        .return_values(ReturnValue::AllOld).send().await;

      let is_new=match res {
        Err(SdkError::ServiceError {
            err:
                PutItemError {
                    kind: PutItemErrorKind::ConditionalCheckFailedException(_),
                    ..
                },
            raw: _,
        }) => {
            false
        },
        Ok(_)=>{
            true
        },
        _ => {return Err(Status::new(tonic::Code::InvalidArgument, "db error"))}
      };


      let mut hasher = Sha256::new();
      hasher.update(self.hash_salt.to_owned()+&open_id);

      let hash = hasher.finalize();


        /*
        {
            item: account_doc.as_hashmap(),
            table_name: String::from("users"),
            condition_expression: Some("attribute_not_exists(Email) and attribute_not_exists(AccountId)".to_string()),
            ..PutItemInput::default()
        };
        */

        let user_id=base85::encode(&hash);
        let payload: JWTPayload = JWTPayload {
            exp:chrono::offset::Local::now().timestamp()+60*60*24*60,
            user_id:user_id,
            open_id:open_id
            
        };

        let token = encode(&Header::default(), &payload, &self.jwt_key)
        .expect("INVALID TOKEN");

        //userid redondant
        let response = login::LoginResponse{
            access_token:token,
            is_new: is_new
        };

        Ok(Response::new(response))
    }

    fn refresh_token< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::RefreshToken> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

    fn logout< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
        //https://developers.google.com/identity/protocols/oauth2/web-server#tokenrevoke
    }

    fn feed< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<feed::FeedRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::ConvHeaderList> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn search< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<search::SearchRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<search::SearchResponse> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn get_informations< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<settings::UserInformationsResponse> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn change_informations< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<settings::UserInformations> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn decline_invitation< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::ResourceRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn block_user< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::BlockRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn unblock_user< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::BlockRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn list_invitations< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::PersonalAssetsRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::ConvHeaderList> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn list_user_convs< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::UserAssetsRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::ConvHeaderList> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn list_user_replies< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::UserAssetsRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::ReplyHeaderList> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn list_user_upvotes< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::UserAssetsRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::ConvHeaderList> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn list_user_downvotes< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::UserAssetsRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::ConvHeaderList> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn get_conv< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<visibility::Visibility> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn modify_visibility< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<visibility::ModifyVisibilityRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn upvote_conv< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn downvote_conv< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn get_visibility< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<conversation::Conversation> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn modify_conv< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<conversation::Conversation> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn downvote_reply< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn upvote_reply< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn submit_reply< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<replies::ReplyRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn get_replies< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<replies::GetRepliesRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<replies::ReplyList> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn get_qa_space< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<qa::QaSpace> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn preview_qa_space< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest, > ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<qa::QaSpace> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }


    fn edit_qa_space< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<qa::EditQaSpaceRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

    fn get_notifications< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<notifications::GetNotificationsRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<notifications::NotificationsResponse> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

    fn update_wallet< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedObjectRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

    fn delete_account< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::AuthenticatedRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

    fn feedback< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<user::FeedbackRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::Empty> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

    fn upload_file< 'life0, 'async_trait>(& 'life0 self,request:tonic::Request<common_types::FileUploadRequest> ,) ->  core::pin::Pin<Box<dyn core::future::Future<Output = Result<tonic::Response<common_types::FileUploadResponse> ,tonic::Status, > > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        todo!()
    }

}
